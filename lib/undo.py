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
    """最新 N 条,倒序;不存在文件返回空。"""
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
        op = e["op"]; p = e["payload"]
        if op == "move":
            log("撤销移动 %s: %s ← %s" % (p["folder"], p["from"], p["to"]))
            return move_item(p["to"], p["folder"], p["from"], p.get("emby_id"))
        if op == "delete":
            return {"err": "删除已把 115 文件夹送入回收站,请先去 115 web 还原它,再用「扫描加新内容」补 strm",
                    "lib": p.get("lib"), "folder": p.get("folder"),
                    "hint": "115 web → 回收站 → 找「%s」→ 还原 → 来工具扫这个库" % p.get("folder", "")}
        return {"err": "不支持撤销此操作: " + op}
    return {"err": "未知 undo id: " + undo_id}
