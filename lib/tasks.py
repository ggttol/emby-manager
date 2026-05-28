"""异步任务池:TASKS dict + 线程化 run_async + 取消标志。
长跑(scan/dedup/c115 批量)走 run_async,前端轮询 task_get 看进度。
"""
import threading, time, uuid

from lib.logger import logger

TASKS = {}  # tid -> {tid, kind, status, progress, total, started, ended, result, err, cancelled, status_text}
TASKS_LOCK = threading.Lock()
TASKS_MAX = 100


def task_new(kind):
    tid = uuid.uuid4().hex[:12]
    with TASKS_LOCK:
        TASKS[tid] = {"tid": tid, "kind": kind, "status": "running", "progress": 0, "total": 0,
                      "status_text": "", "started": time.time(), "ended": None,
                      "result": None, "err": None, "cancelled": False}
        if len(TASKS) > TASKS_MAX:
            done = sorted([(t.get("ended") or t["started"], k) for k, t in TASKS.items() if t["status"] != "running"])
            for _, k in done[:len(TASKS) - TASKS_MAX]:
                TASKS.pop(k, None)
    return tid


def task_set(tid, **kw):
    with TASKS_LOCK:
        if tid in TASKS: TASKS[tid].update(kw)


def task_get(tid):
    with TASKS_LOCK:
        return dict(TASKS[tid]) if tid in TASKS else None


def task_cancel(tid):
    with TASKS_LOCK:
        if tid in TASKS and TASKS[tid]["status"] == "running":
            TASKS[tid]["cancelled"] = True; return True
    return False


def task_is_cancelled(tid):
    with TASKS_LOCK:
        return TASKS.get(tid, {}).get("cancelled", False)


def list_tasks(limit=20):
    """返最近 N 个任务(包括 running 和已结束),按 started 倒序。"""
    with TASKS_LOCK:
        items = sorted(TASKS.values(), key=lambda t: t.get("started", 0), reverse=True)[:limit]
        return {"tasks": [dict(t) for t in items]}


def run_async(kind, fn, *args, **kwargs):
    """fn(tid, *args, **kwargs) 返回 result;tid 注入第一个参数,fn 内部可用 task_set / task_is_cancelled。"""
    tid = task_new(kind)
    def wrapper():
        try:
            result = fn(tid, *args, **kwargs)
            with TASKS_LOCK:
                t = TASKS.get(tid)
                if t:
                    t["status"] = "cancelled" if t.get("cancelled") else "done"
                    t["ended"] = time.time(); t["result"] = result
        except Exception as e:
            logger.exception("任务 %s [%s] 失败", tid, kind)
            with TASKS_LOCK:
                t = TASKS.get(tid)
                if t:
                    t["status"] = "error"; t["ended"] = time.time(); t["err"] = str(e)
    threading.Thread(target=wrapper, daemon=True, name="task-%s-%s" % (kind, tid)).start()
    return tid
