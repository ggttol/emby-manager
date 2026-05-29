#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Emby 管理工具 v3 — 跑在 NAS,纯标准库,零依赖,以 root 运行。
功能:仪表盘 / 扫描加新内容 / 一键扫全库 / 海报修复(候选或手填tmdbid) / 去重 / 删除+跨库移动 / 新建库 / 操作日志 / 系统健康
库列表从 Emby 动态读取(新建/外部加的库自动出现)。危险操作先返回清单,前端确认后才执行。
启动: 用 /usr/local/etc/rc.d/emby_manager.sh start   (或 sudo python3 app.py)

模块拆分:配置/日志/鉴权/任务/Emby/c115/业务/Undo 全部在 lib/ 下,本文件只剩 HTTP handler 和入口。
**re-export 段是必须的**(末尾的 `from lib.* import ...`)—— 测试用 `import app; app.qscore` 这样的方式。
"""
import hmac, json, os, re, sys, threading, time, urllib.parse, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from http.cookies import SimpleCookie

# 顺序敏感:config 先(其它 import 时 CFG 已 load)、logger 次、其余按需
from lib.config import (CFG, CFG_LOCK, WEAK_PWS, HERE, CD, STRM, DOCKER, CONFIG_FILE,
                        VE, CURRENT_SCHEMA, MIGRATIONS, _DEFAULTS,
                        _mig_to_v2, load_cfg, save_cfg, migrate_cfg)
from lib.logger import logger, log, LOGS, AppError, START_TIME
from lib.safe import _safe_under
from lib.auth import (TOKENS, TOKENS_LOCK, TOKEN_TTL, LOGIN_FAIL, LOGIN_FAIL_LOCK,
                      LOGIN_WINDOW, LOGIN_MAX_FAIL, SAFE_METHODS,
                      _hash_password, _verify_password,
                      _token_new, _token_drop, _token_csrf, _token_valid,
                      _login_allowed, _login_record_fail, _token_reaper)
from lib.tasks import (TASKS, TASKS_LOCK, TASKS_MAX,
                       task_new, task_set, task_get, task_cancel, task_is_cancelled,
                       list_tasks, run_async)
from lib.emby import (eget, epost, edelete, _url,
                      emby_online, lib_count, fetch_libs, fetch_libs_full,
                      remote_search, apply_match, refresh_series, list_noposter,
                      list_users, create_user, update_user, delete_user)
from lib import c115 as _c115
from lib.c115 import C115_UA, C115_API, _c115_uid, c115_parse_url
from lib.business import (LIB_LOCKS, LIB_LOCKS_GUARD, _lib_lock,
                          qscore, all_libraries, scan_lib, _scan_lib_locked,
                          analyze_dups, subtitle_overview, _del_folder, exec_dedup, delete_item,
                          move_item, _move_item_locked,
                          create_library, list_items, zhuigeng_status, series_gaps,
                          _gb, system_info, get_config, set_config,
                          export_config, import_config, list_strm,
                          scan_all_async, zhuigeng_status_async,
                          fix_poster_batch_async, delete_batch_async,
                          move_batch_async, dedup_exec_batch_async,
                          add_new_pipeline_async, replace_folder, replace_batch_async,
                          zhuigeng_scan_airing_async, zhuigeng_gaps_summary_async,
                          cleanup_suggest_async, dash_todo,
                          cleanup_empty_folders_async, dedup_auto_all_async,
                          gaps_scan_lib_async, detect_mismatched_posters_async,
                          system_health_summary, refresh_no_rating_async)
from lib import business as _biz
from lib.undo import UNDO_FILE, UNDO_MAX, UNDO_LOCK, _undo_record, list_undo, exec_undo
from lib import scheduler as _sched
from lib import catalog as _catalog


# ===== 版本缓存:_version() 读 VERSION 文件并缓存到模块级(每次 do_GET 不要 fs hit) =====
_VERSION_CACHE = None
def _version():
    global _VERSION_CACHE
    if _VERSION_CACHE is None:
        try:
            with open(os.path.join(HERE, "VERSION")) as f:
                _VERSION_CACHE = f.read().strip()
        except Exception:
            _VERSION_CACHE = "unknown"
    return _VERSION_CACHE


# ===== c115 HTTP 包装:_c115_req 留在 app 模块作用域,测试 patch.object(app, "_c115_req", ...) 才能命中 =====
_c115_req = _c115._c115_req


def c115_test(cookie_override=None):
    return _c115.c115_test(_c115_req, cookie_override)


def c115_snap(share_code, receive_code):
    return _c115.c115_snap(_c115_req, share_code, receive_code)


def c115_receive_api(share_code, receive_code, file_ids, target_cid):
    return _c115.c115_receive_api(_c115_req, share_code, receive_code, file_ids, target_cid)


def c115_snap_full(url, pwd):
    return _c115.c115_snap_full(_c115_req, url, pwd)


def c115_list_dirs(cid="0"):
    return _c115.c115_list_dirs(_c115_req, cid)


def c115_auto_cid(max_depth=2):
    return _c115.c115_auto_cid(_c115_req, fetch_libs, max_depth)


def c115_save_to_lib(url, pwd, lib, file_ids=None):
    return _c115.c115_save_to_lib(_c115_req, url, pwd, lib, file_ids)


def c115_save_to_cid(url, pwd, cid, label=None, file_ids=None):
    return _c115.c115_save_to_cid(_c115_req, url, pwd, cid, label, file_ids)


def c115_offline_add(url, cid, label=None):
    return _c115.c115_offline_add(_c115_req, url, cid, label)


# business 里的 c115 批处理也走本模块的 c115_snap_full / c115_save_to_lib(让 patch 链能贯穿)
def c115_snap_batch(items, default_pwd=""):
    return _biz.c115_snap_batch(c115_snap_full, items, default_pwd)


def c115_save_batch(items, lib, default_pwd=""):
    return _biz.c115_save_batch(c115_save_to_lib, items, lib, default_pwd)


def c115_snap_batch_async(tid, items, default_pwd=""):
    return _biz.c115_snap_batch_async(tid, c115_snap_full, items, default_pwd)


def c115_save_batch_async(tid, items, lib, default_pwd=""):
    return _biz.c115_save_batch_async(tid, c115_save_to_lib, items, lib, default_pwd)


# ===== HTTP =====
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _security_headers(self):
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("X-Server-Version", _version())
        # CSP:项目用了大量 inline,允许 'unsafe-inline';禁外部、frame、object;img 允许 data:/https:(海报来自 TMDb)
        self.send_header("Content-Security-Policy",
            "default-src 'self'; img-src 'self' data: https:; "
            "script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; "
            "connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'")
    def _send(self, code, ctype, body, extra_headers=None):
        if isinstance(body, str): body = body.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", ctype); self.send_header("Content-Length", str(len(body)))
        self._security_headers()
        for k, v in (extra_headers or []):
            self.send_header(k, v)
        self.end_headers()
        # HEAD 请求:头照发(含 Content-Length),body 不写(do_HEAD 设 _suppress_body)
        if getattr(self, "_suppress_body", False):
            return
        self.wfile.write(body)
    def _json(self, obj, code=200, extra_headers=None):
        self._send(code, "application/json; charset=utf-8", json.dumps(obj, ensure_ascii=False), extra_headers)
    def _cookie_token(self):
        raw = self.headers.get("Cookie", "")
        if not raw: return None
        try:
            c = SimpleCookie(); c.load(raw)
            m = c.get("emby_tok")
            return m.value if m else None
        except Exception:
            return None
    def _client_ip(self):
        """真实客户端 IP(与 login 限流同一套解析:只有直连是受信反代才认 XFF)。"""
        from lib.auth import client_ip_for_login
        remote = self.client_address[0] if self.client_address else ""
        return client_ip_for_login(remote, self.headers.get("X-Forwarded-For", ""), CFG.get("trusted_proxies") or [])
    def _auth(self):
        # 优先 cookie(HttpOnly),向后兼容 X-Token header(老版前端过渡)
        t = self._cookie_token() or self.headers.get("X-Token")
        return _token_valid(t, ip=self._client_ip())
    def _csrf_ok(self):
        """非 safe method:校验 X-CSRF-Token 匹配 token 对应的 csrf。"""
        if self.command in SAFE_METHODS: return True
        t = self._cookie_token() or self.headers.get("X-Token")
        if not t: return False
        sent = self.headers.get("X-CSRF-Token", "")
        expected = _token_csrf(t)
        return bool(expected) and hmac.compare_digest(sent, expected)
    def _body(self):
        ln = int(self.headers.get("Content-Length", 0) or 0)
        try: return json.loads(self.rfile.read(ln).decode("utf-8")) if ln else {}
        except Exception: return {}
    def do_GET(self):
        u = urllib.parse.urlparse(self.path); path = u.path
        if path in ("/", "/index.html"):
            try: html = open(os.path.join(HERE, "index.html"), encoding="utf-8").read()
            except Exception: html = "<h1>缺少 index.html</h1>"
            return self._send(200, "text/html; charset=utf-8", html)
        # 公开静态资源:iOS 主屏图标 / favicon / PWA manifest。白名单路径(不开 /static/* 通配防遍历)
        STATIC_MAP = {
            "/apple-touch-icon.png":             ("static/apple-touch-icon.png", "image/png"),
            "/apple-touch-icon-precomposed.png": ("static/apple-touch-icon.png", "image/png"),  # 老 iOS 兜底名
            "/favicon.ico":                       ("static/favicon-32.png",       "image/png"),
            "/favicon.png":                       ("static/favicon-32.png",       "image/png"),
            "/icon-192.png":                      ("static/icon-192.png",         "image/png"),
            "/icon-512.png":                      ("static/icon-512.png",         "image/png"),
            "/manifest.json":                     ("static/manifest.json",        "application/manifest+json; charset=utf-8"),
        }
        if path in STATIC_MAP:
            relpath, ctype = STATIC_MAP[path]
            full = os.path.join(HERE, relpath)
            try:
                with open(full, "rb") as f: data = f.read()
            except Exception:
                return self._send(404, "text/plain", "not found")
            # 长缓存(图标极少改;改了重启服务时 ETag 也变)
            return self._send(200, ctype, data, extra_headers=[("Cache-Control", "public, max-age=86400")])
        if path == "/health":
            # 公开,不要 auth — 给外部探活(uptime kuma 之类)用
            emb = emby_online()
            return self._json({"status": "ok", "version": _version(), "uptime": int(time.time() - START_TIME),
                               "emby_online": emb.get("online", False),
                               "c115_cookie_set": bool(CFG.get("c115_cookie")),
                               "cd_mounted": os.path.isdir(CD) and bool(os.listdir(CD) if os.path.isdir(CD) else [])})
        if path.startswith("/api/"):
            if not self._auth(): return self._json({"err": "未登录"}, 401)
            q = urllib.parse.parse_qs(u.query)
            if path == "/api/libraries":
                _, excluded = fetch_libs_full()
                return self._json({"emby": emby_online(), "libs": all_libraries(), "excluded": excluded})
            if path == "/api/system": return self._json(system_info())
            if path == "/api/system/health": return self._json(system_health_summary())
            if path == "/api/noposter": return self._json({"items": list_noposter()})
            if path == "/api/dups": return self._json(analyze_dups())
            if path == "/api/subtitles": return self._json(subtitle_overview())
            if path == "/api/items": return self._json(list_items(q.get("lib", [""])[0]))
            if path == "/api/zhuigeng": return self._json(zhuigeng_status())
            if path == "/api/gaps": return self._json(series_gaps(q.get("id", [""])[0]))
            if path == "/api/refreshseries": return self._json(refresh_series(q.get("id", [""])[0]))
            if path == "/api/log": return self._json({"logs": list(LOGS)[:200]})
            if path == "/api/users":
                with_act = q.get("withActivity", ["0"])[0] in ("1", "true", "True")
                return self._json({"users": list_users(with_act)})
            if path == "/api/config": return self._json(get_config())
            if path == "/api/search": return self._json({"candidates": remote_search(q.get("id", [""])[0], q.get("name", [""])[0], q.get("type", ["Series"])[0])})
            if path == "/api/c115/test": return self._json(c115_test())
            if path == "/api/c115/auto_cid": return self._json(c115_auto_cid())
            if path == "/api/task":
                t = task_get(q.get("tid", [""])[0])
                return self._json(t or {"err": "未知任务"}, 200 if t else 404)
            if path == "/api/tasks/list":
                try: lim = max(1, min(200, int(q.get("limit", ["20"])[0])))
                except Exception: lim = 20
                return self._json(list_tasks(lim))
            if path == "/api/strm_list":
                try:
                    return self._json(list_strm(q.get("lib", [""])[0], q.get("folder", [""])[0]))
                except ValueError as e:
                    return self._json({"err": str(e)}, 400)
                except AppError as e:
                    return self._json({"err": e.user_msg}, e.status)
            if path == "/api/config/export":
                cfg = export_config()
                return self._json(cfg, extra_headers=[("Content-Disposition", 'attachment; filename="emby-manager-config.json"')])
            if path == "/api/undo_log": return self._json(list_undo())
            if path == "/api/dash/todo": return self._json(dash_todo())
            if path == "/api/me":
                # 已登录(cookie 通过 _auth) → 返当前 csrf token 给前端 sessionStorage(刷页恢复用)
                t = self._cookie_token() or self.headers.get("X-Token")
                return self._json({"csrf": _token_csrf(t) or "",
                                   "username": CFG.get("username") or "admin"})
            if path == "/api/catalog/search":
                return self._json(_catalog.catalog_search(q.get("q", [""])[0], link_type=q.get("type", [""])[0] or None))
            if path == "/api/catalog/stats":
                return self._json(_catalog.catalog_stats())
            if path == "/api/schedules":
                from lib.business import SCHEDULE_KINDS
                # 顺带把 next_run 算进列表,UI 直接展示;kinds map 也带上避免再多发一次请求
                rows = []
                for s in _sched.list_schedules():
                    nr = _sched.next_run_dt(s)
                    rows.append({**s, "next_run_at": nr.isoformat(timespec="seconds") if nr else None,
                                 "schedule_human": _sched.human_schedule(s.get("schedule") or {}),
                                 "kind_label": SCHEDULE_KINDS.get(s.get("kind"), {}).get("label", s.get("kind"))})
                kinds = [{"kind": k, "label": v["label"], "desc": v.get("desc", "")}
                         for k, v in SCHEDULE_KINDS.items()]
                return self._json({"schedules": rows, "kinds": kinds})
            return self._json({"err": "未知接口"}, 404)
        return self._send(404, "text/plain", "404")
    def do_POST(self):
        path = urllib.parse.urlparse(self.path).path; b = self._body()
        if path == "/api/login":
            # 反代场景:client_address[0] 是反代 IP,真客户端 IP 走 X-Forwarded-For 但只在 client_address 在 trusted_proxies 时才信
            from lib.auth import client_ip_for_login
            remote = self.client_address[0] if self.client_address else ""
            ip = client_ip_for_login(remote, self.headers.get("X-Forwarded-For", ""), CFG.get("trusted_proxies") or [])
            if not _login_allowed(ip):
                return self._json({"err": "登录失败次数过多,5 分钟后再试"}, 429)
            pw = b.get("pw", "") or ""
            stored = CFG.get("password_hash", "")
            if stored and _verify_password(pw, stored):
                t, csrf = _token_new(ip)
                # HttpOnly + SameSite=Strict + Max-Age 7d。
                # 走 HTTPS 反代时(X-Forwarded-Proto=https)动态加 Secure → cookie 不再走明文回传;
                # 纯 HTTP 直连(NAS 内网)不加,否则浏览器不回 cookie 直接登不上。
                secure = "; Secure" if self.headers.get("X-Forwarded-Proto", "").lower() == "https" else ""
                cookie = "emby_tok=%s; HttpOnly; SameSite=Strict; Path=/; Max-Age=%d%s" % (t, TOKEN_TTL, secure)
                return self._json({"ok": True, "csrf": csrf}, extra_headers=[("Set-Cookie", cookie)])
            _login_record_fail(ip)
            log("登录失败 from %s" % ip)
            return self._json({"err": "密码错误"}, 403)
        if path == "/api/logout":
            t = self._cookie_token() or self.headers.get("X-Token")
            if t: _token_drop(t)
            return self._json({"ok": True}, extra_headers=[("Set-Cookie", "emby_tok=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")])
        if not self._auth(): return self._json({"err": "未登录"}, 401)
        if not self._csrf_ok(): return self._json({"err": "CSRF 校验失败,刷新页面重试"}, 403)
        try:
            if path == "/api/scan":
                if b.get("async"):
                    from lib.business import scan_lib_async
                    return self._json({"tid": run_async("scan_lib", scan_lib_async, b.get("lib"), b.get("keyword"))})
                return self._json(scan_lib(b.get("lib"), b.get("keyword")))
            if path == "/api/fixposter": return self._json(apply_match(b.get("id"), b.get("tmdb"), b.get("type", "Series"), b.get("name", "")))
            if path == "/api/dedup": return self._json(exec_dedup(b.get("tmdb"), b.get("remove", [])))
            if path == "/api/move": return self._json(move_item(b.get("from"), b.get("folder"), b.get("to"), b.get("id"), on_conflict=b.get("on_conflict", "error")))
            if path == "/api/createlib": return self._json(create_library(b.get("name"), b.get("ctype")))
            if path == "/api/users/new": return self._json(create_user(b.get("name"), b.get("pw")))
            if path == "/api/users/update": return self._json(update_user(b.get("id"), b.get("maxsessions"), b.get("disabled"), b.get("bitrate_mbps")))
            if path == "/api/config": return self._json(set_config(b))
            if path == "/api/c115/snap":
                items = b.get("items")
                if items and b.get("async"):
                    return self._json({"tid": run_async("c115_snap_batch", c115_snap_batch_async, items, b.get("pwd", ""))})
                return self._json(c115_snap_batch(items, b.get("pwd", "")) if items else c115_snap_full(b.get("url", ""), b.get("pwd", "")))
            if path == "/api/c115/save":
                items = b.get("items")
                if items and b.get("async"):
                    return self._json({"tid": run_async("c115_save_batch", c115_save_batch_async, items, b.get("lib", ""), b.get("pwd", ""))})
                return self._json(c115_save_batch(items, b.get("lib", ""), b.get("pwd", "")) if items else c115_save_to_lib(b.get("url", ""), b.get("pwd", ""), b.get("lib", ""), b.get("file_ids")))
            if path == "/api/scan_all":
                return self._json({"tid": run_async("scan_all", scan_all_async)})
            # /api/zhuigeng POST async 路由暂时去除(前端未接,v3.0.x follow-up 时同步接上;
            # 同步 GET 版仍在,zhuigeng_status_async 业务函数保留供 follow-up 直接接路由)
            if path == "/api/fixposter_batch":
                return self._json({"tid": run_async("fixposter_batch", fix_poster_batch_async,
                                                   b.get("ids") or [], b.get("type", "Series"))})
            if path == "/api/manage/delete_batch":
                return self._json({"tid": run_async("delete_batch", delete_batch_async,
                                                   b.get("lib", ""), b.get("items") or [])})
            if path == "/api/manage/move_batch":
                return self._json({"tid": run_async("move_batch", move_batch_async,
                                                   b.get("from", ""), b.get("to", ""), b.get("items") or [],
                                                   b.get("on_conflict", "error"))})
            if path == "/api/dedup/exec_batch":
                return self._json({"tid": run_async("dedup_exec_batch", dedup_exec_batch_async,
                                                   b.get("groups") or [])})
            if path == "/api/catalog/transfer":
                # 从资源库选中 → 按链接类型自动路由:115 分享链走转存(秒传);magnet/ed2k 走离线下载
                lib = b.get("lib"); cid = b.get("cid")
                url = b.get("link") or b.get("url", ""); pwd = b.get("pwd", "")
                ul = (url or "").lower()
                is_offline = ul.startswith("magnet:") or ul.startswith("ed2k:") or \
                             (ul.startswith("http") and "/s/" not in ul)  # 非 115 分享的 http 直链也走离线
                # 解析目标 cid:显式 cid 优先,否则用 lib 映射的 cid
                target_cid = None; label = b.get("label") or lib
                if cid not in (None, ""):
                    target_cid = str(cid)
                elif lib:
                    target_cid = (CFG.get("c115_cid_map") or {}).get(lib)
                    if not target_cid:
                        return self._json({"ok": False, "err": "库「%s」没配 115 cid,去设置页填" % lib}, 400)
                else:
                    return self._json({"ok": False, "err": "未指定目标库或 cid"}, 400)
                # 正整数校验:对【两条路径都生效】(显式 cid 和库映射来的 cid)。
                # 防误配的 cid_map 值("0"=115 根目录 或 非数字)让离线下载(真下载/占配额)灌进根目录(review)
                if not re.fullmatch(r"[1-9]\d*", str(target_cid)):
                    return self._json({"ok": False, "err": "目标 cid 非法(必须正整数,0=根目录不允许;检查库的 cid 配置)"}, 400)
                if is_offline:
                    return self._json(c115_offline_add(url, target_cid, label=label))
                return self._json(c115_save_to_cid(url, pwd, target_cid, label=label))
            if path == "/api/c115/test_candidate":
                return self._json(c115_test(b.get("cookie")))
            if path == "/api/c115/auto_cid":
                return self._json({"tid": run_async("c115_auto_cid",
                                                   lambda tid, depth=2: c115_auto_cid(depth),
                                                   b.get("max_depth", 2))})
            if path == "/api/config/import":
                return self._json(import_config(b))
            if path == "/api/dedup/replace":
                # 全替换:删 lose folder + 如果 win 是「lose(N)」格式则改名回 lose
                return self._json(replace_folder(b.get("lib", ""), b.get("win_folder", ""), b.get("lose_folder", "")))
            if path == "/api/dedup/replace_batch":
                # 批量替换 async,返 {tid}
                return self._json({"tid": run_async("replace_batch", replace_batch_async, b.get("items") or [])})
            if path == "/api/zhuigeng/scan_airing":
                # 一键扫所有在更剧 → 报告
                return self._json({"tid": run_async("zhuigeng_scan_airing", zhuigeng_scan_airing_async)})
            if path == "/api/zhuigeng/gaps_summary":
                # 汇总所有在更剧的缺集 → 求资源清单
                return self._json({"tid": run_async("zhuigeng_gaps_summary", zhuigeng_gaps_summary_async)})
            if path == "/api/cleanup/suggest":
                # 智能清理建议:多维度分析某库;dimensions=[rating,age,idle,size,meta] 子集
                lib = b.get("lib", "")
                top = max(10, min(500, int(b.get("top", 80))))
                min_score = max(0, min(200, int(b.get("min_score", 20))))
                dims = b.get("dimensions")  # None = 全部维度
                if dims is not None and not isinstance(dims, list):
                    dims = None
                return self._json({"tid": run_async("cleanup_suggest",
                    cleanup_suggest_async, lib, top, min_score, dims)})
            if path == "/api/cleanup/empty_folders":
                # 扫某库的 115 上无视频文件的空 folder
                return self._json({"tid": run_async("cleanup_empty_folders",
                    cleanup_empty_folders_async, b.get("lib", ""))})
            if path == "/api/cleanup/refresh_no_rating":
                # 对该库无评分剧触发 emby 元数据刷新(补 TMDb 评分)
                return self._json({"tid": run_async("refresh_no_rating",
                    refresh_no_rating_async, b.get("lib", ""))})
            if path == "/api/dedup/auto_all":
                # 一键处理 analyze_dups 的所有 auto dups(不进 review)
                return self._json({"tid": run_async("dedup_auto_all", dedup_auto_all_async)})
            if path == "/api/gaps/scan_lib":
                # 全库缺集扫描:对该剧集库每部剧查缺集
                return self._json({"tid": run_async("gaps_scan_lib",
                    gaps_scan_lib_async, b.get("lib", ""))})
            if path == "/api/poster/detect_mismatch":
                # 检测疑似绑错 tmdbid(folder 中文 vs emby name 字符重合度低)
                return self._json({"tid": run_async("detect_mismatch", detect_mismatched_posters_async)})
            if path == "/api/wizard/add_new":
                # 一条龙加新资源:批量 receive → 扫涉及库 → 等刮削 → 海报+重复检查 → 报告
                # 必须用 app 模块的 c115_save_to_lib(走 _c115_req 包装链)
                return self._json({"tid": run_async("add_new_pipeline",
                    add_new_pipeline_async, b.get("items") or [], b.get("default_lib", ""),
                    c115_save_to_lib)})
            if path == "/api/schedules/new":
                try:
                    item = _sched.add_schedule(
                        b.get("name") or "", b.get("kind") or "",
                        b.get("schedule") or {}, b.get("params") or {},
                        b.get("enabled", True))
                    return self._json({"ok": True, "schedule": item})
                except ValueError as e:
                    return self._json({"err": str(e)}, 400)
            if path == "/api/schedules/update":
                sid = b.get("id") or ""
                try:
                    item = _sched.update_schedule(sid, {
                        k: b[k] for k in ("name", "params", "schedule", "enabled") if k in b
                    })
                    if not item: return self._json({"err": "找不到 schedule"}, 404)
                    return self._json({"ok": True, "schedule": item})
                except ValueError as e:
                    return self._json({"err": str(e)}, 400)
            if path == "/api/schedules/delete":
                return self._json({"ok": _sched.delete_schedule(b.get("id") or "")})
            if path == "/api/schedules/run":
                tid = _sched.run_now(b.get("id") or "")
                if not tid: return self._json({"err": "schedule 不存在 / kind 未注册"}, 404)
                return self._json({"ok": True, "tid": tid})
            if path == "/api/task/cancel":
                return self._json({"ok": task_cancel(b.get("tid", ""))})
            if path == "/api/undo":
                return self._json(exec_undo(b.get("id", "")))
            return self._json({"err": "未知接口"}, 404)
        except ValueError as e:  # path traversal / 参数非法
            logger.warning("用户错误 POST %s: %s", path, e)
            return self._json({"err": str(e)}, 400)
        except AppError as e:
            logger.warning("业务错误 POST %s: %s", path, e.user_msg)
            return self._json({"err": e.user_msg}, e.status)
        except Exception as e:
            err_id = uuid.uuid4().hex[:8]
            logger.exception("内部错误 POST %s [errid=%s]", path, err_id)
            return self._json({"err": "内部错误,请把 errid 给运维: " + err_id}, 500)
    def do_HEAD(self):
        # SAFE_METHODS 列了 HEAD(探活/缓存客户端会用),但没实现 handler 会让 BaseHTTP 返 501。
        # 走与 do_GET 相同的路由,只是 _send 时 body 不写出(由 _suppress_body 控制)。
        self._suppress_body = True
        try:
            self.do_GET()
        finally:
            self._suppress_body = False
    def do_DELETE(self):
        if not self._auth(): return self._json({"err": "未登录"}, 401)
        if not self._csrf_ok(): return self._json({"err": "CSRF 校验失败,刷新页面重试"}, 403)
        b = self._body()
        p = urllib.parse.urlparse(self.path).path
        # 与 do_POST 一致的分层脱敏:不把内部异常字符串(可能含 /volume1 路径)原样回客户端
        try:
            if p == "/api/item":
                return self._json(delete_item(b.get("lib"), b.get("folder"), b.get("id")))
            if p == "/api/user":
                return self._json(delete_user(b.get("id")))
            return self._json({"err": "未知接口"}, 404)
        except ValueError as e:
            logger.warning("用户错误 DELETE %s: %s", p, e)
            return self._json({"err": str(e)}, 400)
        except AppError as e:
            logger.warning("业务错误 DELETE %s: %s", p, e.user_msg)
            return self._json({"err": e.user_msg}, e.status)
        except Exception as e:
            err_id = uuid.uuid4().hex[:8]
            logger.exception("内部错误 DELETE %s [errid=%s]", p, err_id)
            return self._json({"err": "内部错误,请把 errid 给运维: " + err_id}, 500)


if __name__ == "__main__":
    migrate_cfg()
    threading.Thread(target=_token_reaper, daemon=True, name="token-reaper").start()
    _sched.start()  # 定时任务循环
    host = CFG.get("host", "127.0.0.1"); port = CFG["port"]
    log("服务启动 @ %s:%d" % (host, port))
    print("Emby 管理工具: http://%s:%d  (schema v%d)" % (host, port, CFG.get("schema_version", 1)), file=sys.stderr)
    ThreadingHTTPServer((host, port), H).serve_forever()
