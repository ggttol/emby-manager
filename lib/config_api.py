"""HTTP-facing config read/write/export/import helpers."""
import copy
import time

from lib.config import CFG
from lib.emby import emby_online
from lib.logger import AppError, log


def get_config():
    ck = CFG.get("c115_cookie", "")
    mask = (ck[:18] + "…" + ck[-18:]) if len(ck) > 50 else ck
    from lib.config import _DEF_CD, _DEF_STRM, _DEF_DOCKER
    return {"emby_url": CFG["emby_url"], "api_key": CFG["api_key"], "port": CFG["port"],
            "c115_cookie_set": bool(ck), "c115_cookie_mask": mask,
            "c115_cid_map": CFG.get("c115_cid_map") or {},
            "trusted_proxies": CFG.get("trusted_proxies") or [],
            "auto_strm_enabled": bool(CFG.get("auto_strm_enabled")),
            "auto_strm_fullauto": bool(CFG.get("auto_strm_fullauto")),
            "cd2_mount_prefix": CFG.get("cd2_mount_prefix") or "/CloudNAS/CloudDrive",
            "auto_strm_debounce_sec": CFG.get("auto_strm_debounce_sec", 8),
            "cd2_webhook_secret_set": bool(CFG.get("cd2_webhook_secret")),
            "cd": CFG.get("cd") or _DEF_CD, "strm": CFG.get("strm") or _DEF_STRM,
            "docker": CFG.get("docker") or _DEF_DOCKER}


def set_config(b):
    from lib.config import CFG_LOCK, WEAK_PWS, save_cfg
    from lib.auth import _hash_password, _verify_password
    changed = []
    password_changed = False
    path_changed = False
    with CFG_LOCK:
        before = copy.deepcopy(CFG)
        candidate = copy.deepcopy(CFG)
        if b.get("password"):
            pw = b["password"]
            old = b.get("old_password", "")
            cur_hash = candidate.get("password_hash", "")
            if CFG.get("last_password_change_at") and not _verify_password(old, cur_hash):
                raise AppError("旧密码错误", status=403)
            if len(pw) < 6:
                return {"err": "密码至少 6 位"}
            if pw in WEAK_PWS:
                return {"err": "密码在弱密码列表,换一个"}
            candidate["password_hash"] = _hash_password(pw)
            candidate.pop("password", None)
            candidate["last_password_change_at"] = int(time.time())
            changed.append("登录密码")
            password_changed = True
        if b.get("emby_url"):
            candidate["emby_url"] = b["emby_url"].strip(); changed.append("Emby地址")
        if b.get("api_key"):
            candidate["api_key"] = b["api_key"].strip(); changed.append("API Key")
        if b.get("c115_cookie") is not None:
            candidate["c115_cookie"] = b["c115_cookie"].strip(); changed.append("115 Cookie")
        if isinstance(b.get("c115_cid_map"), dict):
            candidate["c115_cid_map"] = {k: str(v).strip() for k, v in b["c115_cid_map"].items() if str(v).strip()}
            changed.append("115 库 cid 映射")
        if isinstance(b.get("trusted_proxies"), list):
            candidate["trusted_proxies"] = [str(x).strip() for x in b["trusted_proxies"] if str(x).strip()]
            changed.append("受信反代 IP")
        if b.get("cd2_webhook_secret") is not None:
            candidate["cd2_webhook_secret"] = str(b["cd2_webhook_secret"]).strip(); changed.append("CD2 webhook 密钥")
        if b.get("cd2_mount_prefix") is not None and str(b["cd2_mount_prefix"]).strip():
            v = str(b["cd2_mount_prefix"]).strip()
            if not v.startswith("/"):
                return {"err": "CD2 挂载前缀必须以 / 开头: %r" % v}
            candidate["cd2_mount_prefix"] = v.rstrip("/") or "/"; changed.append("CD2 挂载前缀")
        if b.get("auto_strm_enabled") is not None:
            candidate["auto_strm_enabled"] = bool(b["auto_strm_enabled"]); changed.append("自动 strm 开关")
        if b.get("auto_strm_fullauto") is not None:
            candidate["auto_strm_fullauto"] = bool(b["auto_strm_fullauto"]); changed.append("自动 strm 全自动")
        if b.get("auto_strm_debounce_sec") is not None:
            try:
                candidate["auto_strm_debounce_sec"] = max(1, min(120, int(b["auto_strm_debounce_sec"])))
                changed.append("防抖窗口")
            except Exception:
                pass
        for k, name in (("cd", "115 挂载根"), ("strm", "strm 根"), ("docker", "docker 路径")):
            v = b.get(k)
            if v is not None and str(v).strip():
                v = str(v).strip()
                if not v.startswith("/"):
                    return {"err": "%s 必须是绝对路径(以 / 开头): %r" % (name, v)}
                candidate[k] = v; changed.append(name); path_changed = True
        CFG.clear(); CFG.update(candidate)
        if not save_cfg():
            CFG.clear(); CFG.update(before)
            raise AppError("配置保存失败(磁盘空间或权限异常),未应用本次修改", status=500)
    if password_changed:
        try:
            from lib.auth import TOKENS, TOKENS_LOCK
            with TOKENS_LOCK:
                TOKENS.clear()
        except Exception:
            pass
    if path_changed:
        try:
            from lib.config import _apply_paths
            _apply_paths()
        except Exception:
            pass
    log("修改配置: " + "、".join(changed))
    r = {"ok": True, "changed": changed, "emby": emby_online()}
    if path_changed:
        r["restart_needed"] = True
        r["note"] = "存储路径已存,但扫描/删除等用到路径的功能要【重启服务】才生效"
    return r


SENSITIVE_KEYS = ("password_hash", "c115_cookie", "cd2_webhook_secret")
PROTECTED_IMPORT_KEYS = ("schema_version", "password_hash", "c115_cookie",
                         "last_password_change_at", "username",
                         "host", "trusted_proxies", "cd2_webhook_secret")
IMPORTABLE_CONFIG_KEYS = frozenset((
    "emby_url", "api_key", "port", "cd", "strm", "docker", "c115_cid_map", "schedules",
    "auto_strm_enabled", "auto_strm_fullauto", "auto_strm_debounce_sec", "cd2_mount_prefix",
    "bind_token_ip",
))


def _normalize_import_value(key, value):
    if key == "port":
        if isinstance(value, bool):
            raise AppError("port 必须是 1-65535 的整数", status=400)
        try:
            port = int(value)
        except (TypeError, ValueError):
            raise AppError("port 必须是 1-65535 的整数", status=400)
        if not 1 <= port <= 65535:
            raise AppError("port 必须是 1-65535 的整数", status=400)
        return port
    if key in ("cd", "strm", "docker"):
        if not isinstance(value, str) or not value.startswith("/"):
            raise AppError("%s 必须是绝对路径" % key, status=400)
        return value.strip()
    if key == "emby_url":
        if not isinstance(value, str) or not value.strip().startswith(("http://", "https://")):
            raise AppError("emby_url 必须以 http:// 或 https:// 开头", status=400)
        return value.strip().rstrip("/")
    if key == "api_key":
        if not isinstance(value, str):
            raise AppError("api_key 必须是字符串", status=400)
        return value.strip()
    if key == "c115_cid_map":
        if not isinstance(value, dict):
            raise AppError("c115_cid_map 必须是对象", status=400)
        return {str(k): str(v).strip() for k, v in value.items() if str(v).strip()}
    if key == "schedules":
        if not isinstance(value, list):
            raise AppError("schedules 必须是数组", status=400)
        from lib.scheduler import _validate_schedule
        for row in value:
            if not isinstance(row, dict):
                raise AppError("schedules 每项必须是对象", status=400)
            try:
                _validate_schedule(row.get("schedule") or {})
            except ValueError as e:
                raise AppError("定时任务配置非法: " + str(e), status=400)
        return value
    if key in ("auto_strm_enabled", "auto_strm_fullauto", "bind_token_ip"):
        if not isinstance(value, bool):
            raise AppError("%s 必须是 true/false" % key, status=400)
        return value
    if key == "auto_strm_debounce_sec":
        if isinstance(value, bool):
            raise AppError("auto_strm_debounce_sec 必须是 1-120 的整数", status=400)
        try:
            sec = int(value)
        except (TypeError, ValueError):
            raise AppError("auto_strm_debounce_sec 必须是 1-120 的整数", status=400)
        if not 1 <= sec <= 120:
            raise AppError("auto_strm_debounce_sec 必须是 1-120 的整数", status=400)
        return sec
    if key == "cd2_mount_prefix":
        if not isinstance(value, str) or not value.startswith("/"):
            raise AppError("cd2_mount_prefix 必须是绝对路径", status=400)
        return value.rstrip("/") or "/"
    raise AppError("不支持导入配置字段: " + str(key), status=400)


def export_config():
    from lib.config import CFG as _CFG, CFG_LOCK
    with CFG_LOCK:
        snapshot = list(_CFG.items())
    out = {}
    for k, v in snapshot:
        out[k] = "<redacted>" if k in SENSITIVE_KEYS and v else v
    return out


def import_config(b):
    from lib.config import CFG as _CFG, CFG_LOCK, save_cfg, CURRENT_SCHEMA
    if not b.get("confirm"):
        raise AppError("必须显式 confirm=true", status=400)
    cfg = b.get("cfg") or {}
    if not isinstance(cfg, dict):
        raise AppError("cfg 必须是 dict", status=400)
    sv = cfg.get("schema_version")
    if sv is not None and sv != CURRENT_SCHEMA:
        raise AppError("schema 不匹配:导入 %s vs 当前 %s" % (sv, CURRENT_SCHEMA), status=400)
    applied = []
    skipped_protected = []
    skipped_unknown = []
    with CFG_LOCK:
        before = copy.deepcopy(_CFG)
        candidate = copy.deepcopy(_CFG)
        for k, v in cfg.items():
            if k in PROTECTED_IMPORT_KEYS:
                skipped_protected.append(k)
                continue
            if k not in IMPORTABLE_CONFIG_KEYS:
                skipped_unknown.append(k)
                continue
            candidate[k] = _normalize_import_value(k, v)
            applied.append(k)
        _CFG.clear(); _CFG.update(candidate)
        if not save_cfg():
            _CFG.clear(); _CFG.update(before)
            raise AppError("配置保存失败(磁盘空间或权限异常),未应用导入", status=500)
    log("config 导入: 改 %d 字段 [%s]%s" % (
        len(applied), ", ".join(applied),
        (" · 拒受保护字段 " + ",".join(skipped_protected) if skipped_protected else "") +
        (" · 跳未知字段 " + ",".join(skipped_unknown) if skipped_unknown else "")))
    return {"ok": True, "applied": applied, "skipped_protected": skipped_protected,
            "skipped_unknown": skipped_unknown}
