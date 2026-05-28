"""定时任务:daily/weekly/monthly 三档触发 + 同周期防重入。

数据形态(CFG["schedules"] 持久化):
  [{"id":"sch_xxx","name":"扫全库","kind":"scan_all","params":{},
    "schedule":{"mode":"daily|weekly|monthly","hour":3,"minute":0,
                "weekday":0-6,"day":1-31},
    "enabled":True,
    "last_run_at":"2026-05-28T03:00:00",   # 本次窗口已 fire 的标记 + UI 展示
    "last_ended_at":"2026-05-28T03:12:00", # 任务结束时间(watch 线程写)
    "last_status":"running|done|error|cancelled",
    "last_tid":"abc123","last_err":"...",
    "created_at":"2026-05-28T20:00:00"}]

触发逻辑(_is_due):now 在 [HH:MM, HH:MM+5min) 窗口里 + 模式日匹配 +
当前周期内 last_run_at 不在 → 触发(单线程轮询 + 5 min 窗口足以兜底 30s tick)。

执行链路:_fire → run_async(business.SCHEDULE_KINDS[kind].fn) → 起 watch 线程
持续写 last_status / last_err 直到任务结束。**business 用 lazy import** 防循环。
"""
import os, threading, time, uuid
from datetime import datetime, timedelta

from lib.config import CFG, CFG_LOCK, save_cfg
from lib.logger import logger, log


# 守护 schedules list:add/update/delete 都过这把锁(CFG_LOCK 是 RLock,可重入)
_LOOP_STOP = threading.Event()
_LOOP_THREAD = None
_POLL_SEC = 30   # 轮询周期;5min 窗口能容下两次 miss


def _now():
    return datetime.now()


def _isofmt(dt):
    return dt.replace(microsecond=0).isoformat()


def _parse_iso(s):
    if not s: return None
    try:
        return datetime.fromisoformat(s)
    except Exception:
        return None


def _ensure_schedules():
    """CFG['schedules'] 不存在或非 list 就置空,避免炸 KeyError。"""
    with CFG_LOCK:
        if not isinstance(CFG.get("schedules"), list):
            CFG["schedules"] = []


# ===== CRUD =====
def list_schedules():
    _ensure_schedules()
    with CFG_LOCK:
        return [dict(s) for s in CFG["schedules"]]


def get_schedule(sid):
    _ensure_schedules()
    with CFG_LOCK:
        for s in CFG["schedules"]:
            if s.get("id") == sid:
                return dict(s)
    return None


def _find_by_id(sid):
    """**调用者必须已持 CFG_LOCK**。返 schedule dict 直接引用(可改)。"""
    for s in CFG.get("schedules", []):
        if s.get("id") == sid:
            return s
    return None


def add_schedule(name, kind, schedule, params=None, enabled=True):
    """schedule={mode,hour,minute,weekday?,day?},返新建 dict。"""
    _validate_schedule(schedule)
    _ensure_schedules()
    now = _isofmt(_now())
    item = {
        "id": "sch_" + uuid.uuid4().hex[:10],
        "name": name or _default_name(kind, schedule),
        "kind": kind,
        "params": params or {},
        "schedule": dict(schedule),
        "enabled": bool(enabled),
        "last_run_at": None,
        "last_ended_at": None,
        "last_status": None,
        "last_tid": None,
        "last_err": None,
        "created_at": now,
    }
    with CFG_LOCK:
        CFG["schedules"].append(item)
        save_cfg()
    log("新增定时 %s [%s] @ %s" % (item["name"], kind, human_schedule(schedule)))
    return dict(item)


def update_schedule(sid, patch):
    """允许改:name / params / schedule / enabled。其余字段忽略。"""
    _ensure_schedules()
    with CFG_LOCK:
        s = _find_by_id(sid)
        if not s:
            return None
        if "name" in patch and patch["name"]:
            s["name"] = patch["name"]
        if "params" in patch:
            s["params"] = patch["params"] or {}
        if "schedule" in patch and patch["schedule"]:
            _validate_schedule(patch["schedule"])
            s["schedule"] = dict(patch["schedule"])
        if "enabled" in patch:
            s["enabled"] = bool(patch["enabled"])
        save_cfg()
        return dict(s)


def delete_schedule(sid):
    _ensure_schedules()
    with CFG_LOCK:
        before = len(CFG["schedules"])
        CFG["schedules"] = [s for s in CFG["schedules"] if s.get("id") != sid]
        ok = len(CFG["schedules"]) < before
        if ok:
            save_cfg()
            log("删除定时 %s" % sid)
        return ok


# ===== 触发判定 =====
def _validate_schedule(sch):
    mode = sch.get("mode")
    if mode not in ("daily", "weekly", "monthly"):
        raise ValueError("mode 必须是 daily/weekly/monthly")
    h = int(sch.get("hour", 0)); m = int(sch.get("minute", 0))
    if not (0 <= h < 24 and 0 <= m < 60):
        raise ValueError("hour/minute 越界")
    if mode == "weekly":
        wd = int(sch.get("weekday", 0))
        if not (0 <= wd <= 6):
            raise ValueError("weekday 必须 0-6(0=周一)")
    if mode == "monthly":
        d = int(sch.get("day", 1))
        if not (1 <= d <= 31):
            raise ValueError("day 必须 1-31")


def human_schedule(sch):
    """生成 UI 友好的描述,用于日志。"""
    wd_names = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
    hm = "%02d:%02d" % (sch.get("hour", 0), sch.get("minute", 0))
    mode = sch.get("mode")
    if mode == "daily": return "每日 " + hm
    if mode == "weekly": return wd_names[sch.get("weekday", 0)] + " " + hm
    if mode == "monthly": return "每月 %d 日 %s" % (sch.get("day", 1), hm)
    return mode


def _default_name(kind, sch):
    return "定时 %s @ %s" % (kind, human_schedule(sch))


def is_due(s, now=None):
    """now 在调度窗口内 + 当前周期未跑过 → True。"""
    now = now or _now()
    sch = s.get("schedule") or {}
    try:
        _validate_schedule(sch)
    except Exception:
        return False
    H, M = sch["hour"], sch["minute"]
    target = now.replace(hour=H, minute=M, second=0, microsecond=0)
    delta = (now - target).total_seconds()
    if not (0 <= delta < 300):   # 5 分钟窗口
        return False
    mode = sch["mode"]
    if mode == "weekly" and now.weekday() != int(sch.get("weekday", 0)):
        return False
    if mode == "monthly" and now.day != int(sch.get("day", 1)):
        return False
    # 同周期防重入
    lr = _parse_iso(s.get("last_run_at"))
    if lr:
        if mode == "daily" and lr.date() == now.date():
            return False
        if mode == "weekly":
            # 同 ISO 周(年 + 周号)算同周期
            if lr.isocalendar()[:2] == now.isocalendar()[:2]:
                return False
        if mode == "monthly" and lr.year == now.year and lr.month == now.month:
            return False
    return True


def next_run_dt(s, now=None):
    """返回下一次"应该跑"的时间(UI 展示用)。"""
    now = now or _now()
    sch = s.get("schedule") or {}
    try:
        _validate_schedule(sch)
    except Exception:
        return None
    H, M = sch["hour"], sch["minute"]
    today_t = now.replace(hour=H, minute=M, second=0, microsecond=0)
    mode = sch["mode"]
    if mode == "daily":
        return today_t if today_t > now else today_t + timedelta(days=1)
    if mode == "weekly":
        wd = int(sch.get("weekday", 0))
        diff = (wd - now.weekday()) % 7
        cand = today_t + timedelta(days=diff)
        if cand <= now:
            cand += timedelta(days=7)
        return cand
    if mode == "monthly":
        day = int(sch.get("day", 1))
        # 本月候选(超过本月长度时夹到月末)
        import calendar
        last = calendar.monthrange(now.year, now.month)[1]
        cand = today_t.replace(day=min(day, last))
        if cand > now:
            return cand
        # 下个月
        ny, nm = (now.year + 1, 1) if now.month == 12 else (now.year, now.month + 1)
        last2 = calendar.monthrange(ny, nm)[1]
        return today_t.replace(year=ny, month=nm, day=min(day, last2))
    return None


# ===== 执行 =====
def _fire(sid):
    """触发某 schedule;立即标记 last_run_at + last_status=running,然后挂 watch 线程跟踪结束。
    防重叠:上次任务还 running 时跳过(避免长跑任务跨周期 + 同 schedule 并发跑两份)。"""
    from lib import business  # lazy:business → tasks 链,避免模块级循环
    from lib.tasks import run_async, task_get

    with CFG_LOCK:
        s = _find_by_id(sid)
        if not s:
            return None
        # 重叠保护:上一次还在跑 → 不并发起新的(scheduler/run_now 都过 _fire,统一防御)
        if s.get("last_status") == "running":
            logger.warning("定时 %s [%s] 上次任务仍 running,跳过本次触发", sid, s.get("kind"))
            return None
        kind = s.get("kind")
        spec = business.SCHEDULE_KINDS.get(kind)
        if not spec:
            s["last_status"] = "error"; s["last_err"] = "未知 kind: " + str(kind)
            save_cfg()
            logger.warning("定时 %s 未知 kind=%s", sid, kind)
            return None
        # 复刻一份 spec 信息,出锁后用
        fn = spec["fn"]
        name = s.get("name", kind)

    tid = run_async("schedule:" + kind, fn)
    now_str = _isofmt(_now())
    with CFG_LOCK:
        s = _find_by_id(sid)
        if s:
            s["last_run_at"] = now_str
            s["last_tid"] = tid
            s["last_status"] = "running"
            s["last_err"] = None
            s["last_ended_at"] = None
            save_cfg()
    log("⏰ 触发定时 %s [%s] → tid=%s" % (name, kind, tid))

    # 监控线程:tasks 没原生 done-callback,自己轮询 + 写 last_*
    def watch():
        deadline = time.time() + 6 * 3600  # 6h 防 watch 卡死
        while time.time() < deadline:
            t = task_get(tid)
            if not t:
                break
            if t["status"] != "running":
                with CFG_LOCK:
                    s2 = _find_by_id(sid)
                    if s2 and s2.get("last_tid") == tid:
                        s2["last_status"] = t["status"]
                        s2["last_err"] = t.get("err")
                        s2["last_ended_at"] = _isofmt(_now())
                        save_cfg()
                log("⏰ 定时 %s 结束 [%s] · tid=%s" % (name, t["status"], tid))
                return
            time.sleep(2)
        # 超时(任务跑了 >6h):标超时,不再 watch
        with CFG_LOCK:
            s2 = _find_by_id(sid)
            if s2 and s2.get("last_tid") == tid:
                s2["last_status"] = "watch_timeout"
                s2["last_err"] = "watch 超时 6h"
                save_cfg()
    threading.Thread(target=watch, daemon=True, name="sch-watch-" + sid).start()
    return tid


def run_now(sid):
    """用户点'立即执行':绕过 is_due 直接 _fire。"""
    return _fire(sid)


def _loop():
    """daemon thread 主循环:30s 一轮,对所有 enabled 且 is_due 的 schedule fire。"""
    log("⏰ scheduler 启动 (轮询 %ds)" % _POLL_SEC)
    # 启动时先 sleep 一下让 app 初始化完
    if _LOOP_STOP.wait(5):
        return
    while not _LOOP_STOP.is_set():
        try:
            now = _now()
            for s in list_schedules():
                if not s.get("enabled"):
                    continue
                if is_due(s, now):
                    try:
                        _fire(s["id"])
                    except Exception as e:
                        logger.exception("定时 %s 触发失败", s.get("id"))
        except Exception:
            logger.exception("scheduler 循环异常(继续)")
        if _LOOP_STOP.wait(_POLL_SEC):
            return
    log("⏰ scheduler 停止")


def start():
    global _LOOP_THREAD
    if _LOOP_THREAD and _LOOP_THREAD.is_alive():
        return
    _LOOP_STOP.clear()
    _LOOP_THREAD = threading.Thread(target=_loop, daemon=True, name="scheduler")
    _LOOP_THREAD.start()


def stop():
    _LOOP_STOP.set()
