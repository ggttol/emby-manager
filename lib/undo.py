"""Undo 日志:jsonl 文件,append-only,UNDO_MAX 条 truncate 旧的。
delete 不可逆(只能提示用户去 115 还原);move 是可反向 move 调用。
"""
import json, os, threading, time, uuid

from lib.config import HERE
from lib.logger import logger, log


UNDO_FILE = os.path.join(HERE, "undo_log.jsonl")
UNDO_MAX = 200
UNDO_LOCK = threading.Lock()


def _undo_record(op, payload):
    """delete/move 落 jsonl,每行一条;到 UNDO_MAX 条 truncate 旧的。"""
    try:
        entry = {"id": uuid.uuid4().hex[:8], "ts": int(time.time()), "op": op, "payload": payload}
        with UNDO_LOCK:
            with open(UNDO_FILE, "a", encoding="utf-8") as f:
                f.write(json.dumps(entry, ensure_ascii=False) + "\n")
            try:
                with open(UNDO_FILE, encoding="utf-8") as f:
                    lines = f.readlines()
                if len(lines) > UNDO_MAX:
                    with open(UNDO_FILE, "w", encoding="utf-8") as f:
                        f.writelines(lines[-UNDO_MAX:])
                    os.chmod(UNDO_FILE, 0o600)
            except Exception:
                logger.exception("undo log 自截失败")
    except Exception:
        logger.exception("写 undo log 失败")


def list_undo(limit=50):
    """最新 N 条,倒序;不存在文件返回空。带 undone 标记(已撤销过的)。"""
    try:
        with UNDO_LOCK:
            with open(UNDO_FILE, encoding="utf-8") as f:
                lines = f.readlines()
        out = []
        for ln in reversed(lines[-limit:]):
            try: out.append(json.loads(ln))
            except Exception: continue
        return {"items": out}
    except FileNotFoundError:
        return {"items": []}


def _mark_undone(undo_id):
    """把某条 undo 记录标记为 undone=true(改写那一行),防 move 撤销被反复点导致来回搬。"""
    try:
        with UNDO_LOCK:
            with open(UNDO_FILE, encoding="utf-8") as f:
                lines = f.readlines()
            changed = False
            for i, ln in enumerate(lines):
                try: e = json.loads(ln)
                except Exception: continue
                if e.get("id") == undo_id and not e.get("undone"):
                    e["undone"] = True
                    lines[i] = json.dumps(e, ensure_ascii=False) + "\n"; changed = True; break
            if changed:
                with open(UNDO_FILE, "w", encoding="utf-8") as f:
                    f.writelines(lines)
    except Exception:
        logger.exception("标记 undone 失败")


def exec_undo(undo_id):
    """按 id 撤销。move 可直接反向调用;delete 只能提示用户去 115 还原后重扫。"""
    # lazy import:undo → business → undo 循环风险
    from lib.business import move_item
    try:
        with UNDO_LOCK:
            with open(UNDO_FILE, encoding="utf-8") as f:
                lines = f.readlines()
    except FileNotFoundError:
        return {"err": "无 undo 记录"}
    for ln in reversed(lines):
        try: e = json.loads(ln)
        except Exception: continue
        if e.get("id") != undo_id:
            continue
        if e.get("undone"):
            return {"err": "这条操作已经撤销过了,别重复点(否则会来回搬)"}
        op = e["op"]; p = e["payload"]
        if op == "move":
            log("撤销移动 %s: %s ← %s" % (p["folder"], p["from"], p["to"]))
            r = move_item(p["to"], p["folder"], p["from"], p.get("emby_id"))
            if not r.get("err"):
                _mark_undone(undo_id)
            return r
        if op == "rebind":
            # 改绑 tmdbid 的撤销 = 重新绑回旧 tmdb(旧值为空则没法回滚,提示去海报 tab 手动处理)
            old = str(p.get("old_tmdb") or "").strip()
            if not old:
                return {"err": "原来就没绑定 tmdbid,无法自动回滚;请去「海报修复」tab 手动重绑"}
            from lib.emby import apply_match
            try:
                apply_match(p.get("id"), old, p.get("type", "Series"), p.get("name", ""))
                _mark_undone(undo_id)
                log("撤销改绑 %s → 回到 tmdb %s" % (p.get("name") or p.get("id"), old))
                return {"ok": True, "msg": "已改绑回 tmdb %s" % old}
            except Exception as e:
                return {"err": "回滚改绑失败: " + str(e)}
        if op in ("delete", "smart_archive", "replace"):
            # 这三类本质都是「删了某 folder 进 115 回收站」,不能程序反向 —— 统一给回收站还原引导,
            # 而不是丢一句「不支持」让用户以为文件没了(review:smart_archive/replace 之前落到死路)。
            folder = p.get("folder") or p.get("lose_was") or p.get("lose_folder") or ""
            lib = p.get("lib") or p.get("from") or p.get("to") or ""
            label = {"delete": "删除", "smart_archive": "智能归档删源", "replace": "全替换删旧版"}.get(op, op)
            return {"err": "「%s」已把 115 文件夹送入回收站,请先去 115 web 还原它,再用「扫描加新内容」补 strm" % label,
                    "lib": lib, "folder": folder,
                    "hint": "115 web → 回收站 → 找「%s」→ 还原 → 来工具扫这个库" % folder}
        return {"err": "不支持撤销此操作: " + op}
    return {"err": "未知 undo id: " + undo_id}
