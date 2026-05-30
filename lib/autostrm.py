"""autostrm:CloudDrive2 webhook 实时驱动的 strm 自动生成。

链路:
  CD2 挂载文件变化 → CD2 POST /api/cd2/webhook(app.py,密钥保护,立即返回)
    → handle_cd2_event  解析 payload + 路径反映射 + 防抖入队
    → _drain_loop       静默期合并一个 burst 成一次(run_async _process_one_async)
    → business.gen_strm_for_lib_path  生成 strm + 通知 Emby 刷新(复用全库扫描的写入核心)
    → enqueue_match → _match_loop  延迟匹配:复用 business.autostrm_try_match(_fix_poster_one)自动绑 TMDb

设计要点:
  - webhook handler 绝不内联生成(防 CD2 超时重试风暴),只反映射 + 入队 + 立即返回。
  - 只新增、绝不删除 strm(删除/move-out 事件忽略 → mount 抖动不会误删)。
  - 与「增量监控补扫」调度(business.monitor_incremental_async)共享 seen 状态防重复处理。
  - 状态(近期入库 / 未匹配 / 未映射事件)走内存环形缓冲,供 /api/autostrm/status 展示。

模块级只 import config/logger/tasks(都早于本模块加载);business/emby 在函数内 lazy import
(emby.fetch_libs 要打网络,且避免与 business 的潜在循环)。
"""
import collections, json, os, threading, time

from lib.config import CFG, VE, STRM
from lib.logger import logger
from lib.tasks import run_async


# ===== 库 folder → lib 名 缓存(反映射要用;fetch_libs 打 Emby,burst 里别每事件调)=====
_LIBS_CACHE = {"ts": 0.0, "map": {}}
_LIBS_LOCK = threading.Lock()
_LIBS_TTL = 30.0


def _folder_to_lib():
    """{folder: lib_name},30s 缓存。Emby 读失败时回落上次缓存(空也行,反映射会判 None)。"""
    now = time.time()
    with _LIBS_LOCK:
        if _LIBS_CACHE["map"] and now - _LIBS_CACHE["ts"] < _LIBS_TTL:
            return dict(_LIBS_CACHE["map"])
    try:
        from lib.emby import fetch_libs
        m = {meta["folder"]: name for name, meta in fetch_libs().items()}
    except Exception as e:
        logger.warning("autostrm 读库列表失败: %s", e)
        with _LIBS_LOCK:
            return dict(_LIBS_CACHE["map"])
    with _LIBS_LOCK:
        _LIBS_CACHE["ts"] = now
        _LIBS_CACHE["map"] = m
        return dict(m)


def _is_dir(v):
    """CD2 的 is_dir 可能是 bool 或 'True'/'False'/'1' 字符串,统一判真。"""
    return str(v).strip().lower() in ("true", "1", "yes")


def _reverse_map(source_file, f2l=None):
    """CD2 命名空间路径 → (lib_name, top)。剥 cd2_mount_prefix → <folder>/<inner...>,
    folder 必须是已知库 folder,top = inner 第一段。非法/未知 → None。"""
    prefix = (CFG.get("cd2_mount_prefix") or "/CloudNAS/CloudDrive").rstrip("/")
    sf = (source_file or "").replace("\\", "/")
    if not prefix or not sf.startswith(prefix + "/"):
        return None
    rel = sf[len(prefix) + 1:]
    parts = rel.split("/", 1)
    if len(parts) < 2 or not parts[0] or not parts[1]:
        return None
    folder, inner = parts[0], parts[1]
    if f2l is None:
        f2l = _folder_to_lib()
    lib = f2l.get(folder)
    if not lib:
        return None
    top = inner.split("/")[0]
    if not top or top in (".", ".."):
        return None
    return lib, top


# ===== 防抖生成队列 =====
_PENDING = {}                 # (lib, top) -> last_event_ts
_PENDING_LOCK = threading.Lock()
_PENDING_MAX = 5000           # 防 runaway:CD2 抽风狂推时封顶
_STATS = {"dropped": 0}       # 因队列满被丢弃的事件累计(暴露给 status,让用户在 UI 看得到)
_wake = threading.Event()


def _debounce_sec():
    try:
        return max(1, min(120, int(CFG.get("auto_strm_debounce_sec", 8))))
    except Exception:
        return 8


def handle_cd2_event(payload):
    """解析 CD2 webhook payload,反映射 + 入防抖队列。返回接受(入队)的事件数。
    立即返回,绝不内联生成。delete/move-out 事件忽略(防 mount 抖动误删);move 用 destination_file 当新文件。"""
    if not isinstance(payload, dict):
        return 0
    data = payload.get("data")
    if not isinstance(data, list):
        return 0
    f2l = _folder_to_lib()
    accepted = 0
    for ev in data:
        if not isinstance(ev, dict):
            continue
        if _is_dir(ev.get("is_dir")):
            continue
        action = (ev.get("action") or "").strip().lower()
        if action in ("delete", "remove", "deleted", "unlink", "rmdir"):
            continue  # 只新增不删除
        sf = ev.get("destination_file") or ev.get("source_file") or ""
        if not isinstance(sf, str) or not sf.lower().endswith(VE):
            continue  # 非视频(nfo/海报/字幕/垃圾)跳过
        m = _reverse_map(sf, f2l)
        if not m:
            _record_unmapped(sf)
            continue
        lib, top = m
        with _PENDING_LOCK:
            if len(_PENDING) >= _PENDING_MAX and (lib, top) not in _PENDING:
                _STATS["dropped"] += 1
                logger.warning("autostrm 防抖队列已满(%d),丢弃 %s/%s(累计丢弃 %d)",
                               _PENDING_MAX, lib, top, _STATS["dropped"])
                continue
            _PENDING[(lib, top)] = time.time()
        accepted += 1
    if accepted:
        _wake.set()
    return accepted


def _drain_loop():
    """守护线程:某 (lib,top) 静默 debounce 秒后出队;按库分组,每库一个任务(库内多 top 共用一次 Emby 刷新)。"""
    while True:
        _wake.wait(timeout=_debounce_sec())
        _wake.clear()
        try:
            now = time.time()
            deb = _debounce_sec()
            ready = []
            with _PENDING_LOCK:
                for key, ts in list(_PENDING.items()):
                    if now - ts >= deb:
                        ready.append(key)
                        _PENDING.pop(key, None)
            by_lib = {}
            for (lib, top) in ready:
                by_lib.setdefault(lib, []).append(top)
            for lib, tops in by_lib.items():
                run_async("autostrm", _process_lib_batch_async, lib, tops)
        except Exception:
            logger.exception("autostrm drain 单轮异常(已隔离,继续)")


def _process_lib_batch_async(tid, lib, tops):
    """一个 burst 内同一库的多个 top 一起处理:逐 top 生成(各自不刷新),最后对该库 Refresh 一次。
    避免 burst 跨 N 个 top 时把整库刷 N 遍(Emby 反复 reindex)。run_async 调度,出现在任务列表。"""
    from lib import business
    from lib.config import CD
    from lib.emby import fetch_libs, epost
    try:
        meta = fetch_libs().get(lib)
    except Exception:
        meta = None
    total_new = 0; lib_id = None
    for top in tops:
        r = business.gen_strm_for_lib_path(lib, top, do_refresh=False)
        total_new += r.get("new_count", 0)
        if r.get("lib_id"):
            lib_id = r["lib_id"]
        # 记 seen(用 top 实际 mtime),让增量轮询不再重复处理这个 top
        if meta:
            try:
                mt = os.path.getmtime(os.path.join(CD, meta["folder"], top))
                seen_mark(lib, top, mt)
            except Exception:
                logger.warning("autostrm 记 seen mtime 失败 %s/%s(增量轮询可能重复处理)", lib, top, exc_info=True)
        if r.get("needs_match"):
            enqueue_match(lib, top, r.get("lib_id"), r.get("folder"))
        _record(lib, top, new_count=r.get("new_count", 0),
                match=("queued" if r.get("needs_match") else "none"),
                skipped=r.get("skipped"), err=r.get("err"))
    seen_save()
    # 整库刷新一次(只要这批里真有新增 strm)
    if total_new and lib_id:
        try:
            epost("/Items/%s/Refresh" % lib_id,
                  {"Recursive": "true", "MetadataRefreshMode": "Default", "ImageRefreshMode": "Default"})
            logger.info("autostrm[%s] 批量新增 strm %d(%d 个 top)→ 整库刷新一次", lib, total_new, len(tops))
        except Exception:
            logger.warning("autostrm 刷新失败 %s", lib, exc_info=True)
    return {"lib": lib, "tops": len(tops), "new_count": total_new}


# ===== 延迟匹配队列 =====
_MATCH = []                   # [{lib, top, lib_id, folder, attempts, next_ts}]
_MATCH_LOCK = threading.Lock()
_MATCH_TICK = 10              # 匹配 worker 轮询间隔(秒)
_MATCH_FIRST_DELAY = 20       # 首次尝试前等(给 Emby 导入时间)
_MATCH_BACKOFF = 45           # 每次重试退避(秒)
_MATCH_MAX_ATTEMPTS = 10      # 最多尝试(20 + 9*45 ≈ 7min 内等 Emby 导入)


def enqueue_match(lib, top, lib_id, folder):
    """新 top 生成了 strm 但无 tmdbid → 入延迟匹配队列(等 Emby 导入后保守自动绑 TMDb)。
    同 (lib, top) 已在队列则不重复(防 webhook 跨防抖周期 + 轮询重复入队 → 状态虚高/重复匹配)。"""
    if not lib_id or not folder:
        return
    with _MATCH_LOCK:
        if any(e["lib"] == lib and e["top"] == top for e in _MATCH):
            return
        _MATCH.append({"lib": lib, "top": top, "lib_id": lib_id, "folder": folder,
                       "attempts": 0, "next_ts": time.time() + _MATCH_FIRST_DELAY})


def _match_loop():
    """守护线程:整圈包 try/except,单轮异常不让线程永久死掉(否则自动匹配静默停摆)。"""
    while True:
        time.sleep(_MATCH_TICK)
        try:
            _match_tick()
        except Exception:
            logger.exception("autostrm 匹配 worker 单轮异常(已隔离,继续)")


def _match_tick():
    now = time.time()
    due = []
    with _MATCH_LOCK:
        keep = []
        for e in _MATCH:
            (due if e["next_ts"] <= now else keep).append(e)
        _MATCH[:] = keep
    if not due:
        return
    from lib import business
    for e in due:
        try:
            r = business.autostrm_try_match(e["lib_id"], e["folder"], e["top"])
        except Exception as ex:
            r = {"state": "error", "err": str(ex)}
        st = r.get("state")
        if st in ("pending", "error"):
            e["attempts"] += 1
            if e["attempts"] < _MATCH_MAX_ATTEMPTS:
                e["next_ts"] = time.time() + _MATCH_BACKOFF
                with _MATCH_LOCK:
                    _MATCH.append(e)
            else:
                _record(e["lib"], e["top"], match="gave_up", err=r.get("err"))
                _add_unmatched(e["lib"], e["top"], None, None)
        elif st in ("matched", "already"):
            _record(e["lib"], e["top"], match=st, name=r.get("name"))
            _del_unmatched(e["lib"], e["top"])
        elif st == "no_candidate":
            _record(e["lib"], e["top"], match="no_candidate", name=r.get("name"))
            _add_unmatched(e["lib"], e["top"], r.get("id"), r.get("name"))


# ===== seen 状态(与 monitor_incremental 共享,sidecar JSON 落 STRM 根)=====
_SEEN = None                  # {"libs": {lib: {top: mtime}}}
_SEEN_LOCK = threading.Lock()


def _seen_path():
    return os.path.join(STRM, "autostrm_seen.json")


def seen_load():
    global _SEEN
    with _SEEN_LOCK:
        if _SEEN is not None:
            return _SEEN
        try:
            with open(_seen_path(), encoding="utf-8") as f:
                _SEEN = json.load(f)
            if not isinstance(_SEEN, dict):
                _SEEN = {}
        except Exception:
            _SEEN = {}
        _SEEN.setdefault("libs", {})
        return _SEEN


def seen_is_new(name, top, mt):
    """该 top 是否需处理:从没记过 OR mtime 前进了。gen 幂等,首跑对已完整库不会乱生成。"""
    s = seen_load()
    with _SEEN_LOCK:
        stored = s["libs"].get(name, {}).get(top)
    return stored is None or mt > stored


def seen_mark(name, top, mt):
    s = seen_load()
    with _SEEN_LOCK:
        s["libs"].setdefault(name, {})[top] = mt


def seen_save():
    s = seen_load()
    with _SEEN_LOCK:
        try:
            tmp = _seen_path() + ".tmp"
            with open(tmp, "w", encoding="utf-8") as f:
                json.dump(s, f, ensure_ascii=False)
            os.replace(tmp, _seen_path())
        except Exception:
            # 不静默吞:存不下时下次轮询会按旧/空 seen 重复处理(gen 幂等,但会重发 Refresh/重排匹配)
            logger.warning("autostrm seen 状态保存失败(下次轮询可能重复处理): %s", _seen_path(), exc_info=True)


# ===== 状态环形缓冲(给前端 /api/autostrm/status)=====
_RECENT = collections.deque(maxlen=50)     # 近期处理事件
_UNMATCHED = {}                            # (lib,top) -> {lib,top,id,name,ts}
_UNMAPPED = collections.deque(maxlen=20)   # 反映射失败的 source_file(排查前缀配错)
_RB_LOCK = threading.Lock()


def _record(lib, top, **kw):
    with _RB_LOCK:
        row = {"ts": time.time(), "lib": lib, "top": top}
        row.update({k: v for k, v in kw.items() if v is not None})
        _RECENT.appendleft(row)


def _add_unmatched(lib, top, eid, name):
    with _RB_LOCK:
        _UNMATCHED[(lib, top)] = {"lib": lib, "top": top, "id": eid, "name": name, "ts": time.time()}


def _del_unmatched(lib, top):
    with _RB_LOCK:
        _UNMATCHED.pop((lib, top), None)


def _record_unmapped(sf):
    with _RB_LOCK:
        _UNMAPPED.appendleft({"ts": time.time(), "source_file": str(sf)[:300]})


def status():
    """供 /api/autostrm/status:开关 + 队列深度 + 近期入库 + 未匹配 + 未映射样本。"""
    with _RB_LOCK:
        recent = list(_RECENT)
        unmatched = list(_UNMATCHED.values())
        unmapped = list(_UNMAPPED)
    with _PENDING_LOCK:
        pend_gen = len(_PENDING)
    with _MATCH_LOCK:
        pend_match = len(_MATCH)
    return {"enabled": bool(CFG.get("auto_strm_enabled")),
            "fullauto": bool(CFG.get("auto_strm_fullauto")),
            "prefix": CFG.get("cd2_mount_prefix") or "/CloudNAS/CloudDrive",
            "secret_set": bool(CFG.get("cd2_webhook_secret")),
            "pending_gen": pend_gen, "pending_match": pend_match,
            "dropped": _STATS["dropped"],
            "recent": recent, "unmatched": unmatched, "unmapped": unmapped}


# ===== 启动 =====
_STARTED = False
_START_LOCK = threading.Lock()


def start():
    """启动两个守护线程(生成防抖 + 延迟匹配)。app.py __main__ 调一次,幂等。"""
    global _STARTED
    with _START_LOCK:
        if _STARTED:
            return
        _STARTED = True
    seen_load()
    threading.Thread(target=_drain_loop, daemon=True, name="autostrm-drain").start()
    threading.Thread(target=_match_loop, daemon=True, name="autostrm-match").start()
    logger.info("autostrm 守护启动(生成防抖 + 延迟匹配 worker)")
