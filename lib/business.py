"""业务逻辑:扫描、去重、移动、删除、库管理、追更状态、系统信息、c115 批处理。
依赖 lib.emby(eget/epost/...)、lib.config(CFG/STRM/CD/...)、lib.tasks(task_set/...)。

scan_lib / move_item / exec_dedup 走 _lib_lock(name) 串行化,避免并发扫同一库踩对方 strm。
"""
import collections, os, re, shutil, subprocess, threading, time

from lib.config import CFG, CD, STRM, DOCKER, VE
from lib.logger import logger, log, AppError
from lib.safe import _safe_under
from lib.emby import (eget, epost, edelete, emby_online, lib_count,
                      fetch_libs, fetch_libs_full)
from lib.tasks import task_set, task_is_cancelled
from lib.undo import _undo_record


# 库锁:scan/move/dedup 涉及读改 STRM/<lib> 文件树,同库并发会踩对方
LIB_LOCKS = collections.defaultdict(threading.Lock)
LIB_LOCKS_GUARD = threading.Lock()


def _lib_lock(name):
    """获取某 lib 的 Lock(惰性创建,LIB_LOCKS_GUARD 守 dict 写)"""
    with LIB_LOCKS_GUARD:
        return LIB_LOCKS[name]


# ===== 画质评分 =====
def qscore(s):
    """文件名 → 画质分(2160p/1080p/REMUX/HDR/DV 等加成)。返回 int(去重排序键依赖)。"""
    p = s.lower(); sc = 0
    if re.search(r'2160p|\buhd\b|\b4k\b|2160', p): sc += 4000
    elif '1080p' in p or '1080i' in p: sc += 2000
    elif '720p' in p: sc += 1000
    elif '480p' in p or 'dvdrip' in p: sc += 300
    if 'remux' in p: sc += 800
    elif 'bluray' in p or 'blu-ray' in p or 'bdrip' in p: sc += 400
    elif 'web-dl' in p or 'webdl' in p or 'webrip' in p or '.web.' in p: sc += 200
    elif 'hdtv' in p: sc += 100
    if 'dv' in p or '杜比视界' in p or 'dovi' in p: sc += 60
    if 'hdr' in p: sc += 30
    return sc


# ===== 库列表 + 元信息汇总 =====
def all_libraries():
    out = []
    for name, m in fetch_libs().items():
        if m["ctype"] == "tvshows":
            series = lib_count(m["id"], "Series"); ep = lib_count(m["id"], "Episode")
            out.append({"name": name, "id": m["id"], "type": m["ctype"], "count": series, "sub": "%d 部 · %d 集" % (series, ep)})
        else:
            mv = lib_count(m["id"], "Movie")
            out.append({"name": name, "id": m["id"], "type": m["ctype"], "count": mv, "sub": "%d 部影片" % mv})
    return out


# ===== 扫描:生成新 strm + 清孤儿 =====
def scan_lib(name, keyword=None):
    L = fetch_libs()
    if name not in L:
        return {"err": "未知库 " + str(name)}
    lock = _lib_lock(name)
    if not lock.acquire(blocking=False):
        raise AppError("库「%s」已有扫描在跑,等完再试" % name, status=409)
    try:
        return _scan_lib_locked(name, L[name], keyword)
    finally:
        lock.release()


def _scan_lib_locked(name, meta, keyword):
    folder = meta["folder"]
    src_base = os.path.join(CD, folder); strm_base = os.path.join(STRM, folder); media = "/media/" + folder
    if not os.path.isdir(src_base):
        return {"err": "115 文件夹不存在: " + src_base}
    kw = (keyword or "").strip()
    new_files = []; new_folders = {}; attention = []; matched = 0
    for top in sorted(os.listdir(src_base)):
        if kw and kw not in top:          # 填了关键词就只扫匹配的文件夹(快)
            continue
        tp = os.path.join(src_base, top)
        if not os.path.isdir(tp):
            continue
        matched += 1
        missing = []
        for root, ds, fs in os.walk(tp):
            rel = os.path.relpath(root, src_base)
            for f in sorted(fs):
                if f.lower().endswith(VE):
                    sp = os.path.join(strm_base, rel, os.path.splitext(f)[0] + ".strm")
                    if not os.path.exists(sp):
                        missing.append((rel, f))
        if not missing:
            continue
        # 带 tmdbid 的新文件夹,或"已有 strm 的已知文件夹"(如海贼王老剧补新集)→ 照常生成
        if re.search(r'tmdbid[-_]\d+', top, re.IGNORECASE) or os.path.isdir(os.path.join(strm_base, top)):
            for rel, f in missing:
                dd = os.path.join(strm_base, rel); os.makedirs(dd, exist_ok=True)
                with open(os.path.join(dd, os.path.splitext(f)[0] + ".strm"), "w", encoding="utf-8") as w:
                    w.write(media + "/" + os.path.join(rel, f))
                new_files.append(f)
            new_folders[top] = len(missing)
        else:
            attention.append("%s (+%d个视频,无tmdbid且首次出现,需看一眼)" % (top, len(missing)))
    # 顺便清孤儿 strm(指向 /media 已不存在的)—— 解决"换资源后扫描留旧"那种坑
    orphans = 0
    if os.path.isdir(strm_base):
        for root, ds, fs in os.walk(strm_base):
            rel = os.path.relpath(root, strm_base)
            top = rel.split(os.sep)[0] if rel != "." else None
            if kw and top and kw not in top:           # 关键词模式只扫匹配的 top
                continue
            for f in fs:
                if not f.endswith(".strm"):
                    continue
                p = os.path.join(root, f)
                try:
                    content = open(p, encoding="utf-8").read().strip()
                except Exception:
                    continue
                if content.startswith("/media/"):
                    target = CD + content[len("/media"):]
                    if not os.path.exists(target):
                        os.remove(p); orphans += 1
    if new_files or orphans:
        epost("/Items/%s/Refresh" % meta["id"], {"Recursive": "true", "MetadataRefreshMode": "Default", "ImageRefreshMode": "Default"})
        log("扫描[%s] 新增 strm %d,清孤儿 %d" % (name, len(new_files), orphans))
    return {"lib": name, "keyword": kw, "matched": matched, "new_count": len(new_files), "new_folders": new_folders, "attention": attention, "orphans_cleaned": orphans, "refreshed": bool(new_files or orphans)}


# ===== 去重分析 =====
def analyze_dups():
    groups = collections.defaultdict(dict)
    for lib, m in fetch_libs().items():
        base = os.path.join(STRM, m["folder"])
        if not os.path.isdir(base):
            continue
        for top in os.listdir(base):
            tp = os.path.join(base, top)
            if not os.path.isdir(tp):
                continue
            mm = re.search(r'tmdbid[-_](\d+)', top, re.IGNORECASE)
            if not mm:
                continue
            medias = []; maxsc = 0
            for root, ds, fs in os.walk(tp):
                for f in fs:
                    if f.endswith(".strm"):
                        try:
                            c = open(os.path.join(root, f), encoding="utf-8").read().strip()
                        except Exception:
                            c = f
                        medias.append(c); maxsc = max(maxsc, qscore(c))
            groups[mm.group(1)][(lib, top)] = {"medias": medias, "score": maxsc, "n": len(medias)}
    def eps(ms):
        e = set()
        for x in ms:
            # episode 编号最多 4 位(海贼王 1163 集 / 名侦探柯南这类长寿剧)
            z = re.search(r's(\d{1,2})e(\d{1,4})', x.lower())
            if z: e.add((int(z.group(1)), int(z.group(2))))
        return e
    def fmt_eps(es):
        """{(s,e),...} → 'S01·E01-12,45' or 'S01E01-12 · S02E05' for multi-season"""
        if not es: return ""
        by_s = collections.defaultdict(list)
        for s, e in es: by_s[s].append(e)
        def comp(xs):
            xs = sorted(xs); out = []; a = p = xs[0]
            for x in xs[1:]:
                if x == p + 1: p = x
                else: out.append(str(a) if a == p else "%d-%d" % (a, p)); a = p = x
            out.append(str(a) if a == p else "%d-%d" % (a, p))
            return ",".join(out)
        if len(by_s) == 1:
            s = next(iter(by_s))
            return "S%02d · E%s" % (s, comp(by_s[s]))
        return " · ".join("S%02dE%s" % (s, comp(by_s[s])) for s in sorted(by_s))
    dups = []; review = []
    for tid, folders in groups.items():
        if len(folders) < 2:
            continue
        keys = list(folders.keys())
        flat = [x for k in keys for x in folders[k]["medias"]]
        shared = len(flat) != len(set(flat))
        ep_map = {k: eps(folders[k]["medias"]) for k in keys}
        epsets = [ep_map[k] for k in keys]
        is_series = any(len(e) > 0 for e in epsets)
        # 排序:① 画质高优先 ② 同画质看集数(文件数)多优先 ③ 带 (1) 后缀的排后 ④ 名字短优先
        _sfx = lambda s: ('(1)' in s) or (u'（1）' in s)  # 半/全角 (1)
        ranked = sorted(keys, key=lambda k: (-folders[k]["score"], -folders[k]["n"], _sfx(k[1]), len(k[1]), k[1]))
        rows = []
        for k in ranked:
            row = {"lib": k[0], "folder": k[1], "score": folders[k]["score"], "n": folders[k]["n"]}
            if ep_map[k]:
                others = set().union(*[ep_map[k2] for k2 in keys if k2 != k])
                uniq = ep_map[k] - others
                inter = ep_map[k] & others
                row["total_eps"] = len(ep_map[k])
                row["uniq_count"] = len(uniq)
                row["shared_count"] = len(inter)
                row["uniq"] = fmt_eps(uniq) if uniq else ""
            rows.append(row)
        has_zhuigeng = any("追更" in k[0] for k in keys)
        if shared:
            review.append({"tmdb": tid, "why": "多库共享同一文件(删文件会双双坏)", "rows": rows})
        elif has_zhuigeng:
            # 追更库通常是用户故意分版本(完结组+追更组/不同画质各更新中),自动删风险大
            review.append({"tmdb": tid, "why": "追更库,可能是用户故意保留多版本,请手动确认", "rows": rows})
        elif is_series:
            ne = [e for e in epsets if e]
            inter = set.intersection(*ne) if len(ne) == len(keys) and ne else set()
            sm = min((len(e) for e in ne), default=0)
            if inter and sm and len(inter) >= 0.5 * sm:
                # 集数倒挂保护:拟保留版本集数 < 待删版本集数 → 进 review(不自动删)
                keep_n = rows[0]["n"]; max_rm_n = max((r["n"] for r in rows[1:]), default=0)
                if keep_n < max_rm_n:
                    review.append({"tmdb": tid, "why": "拟保留版本集数(%d)少于待删版本(%d),手动确认" % (keep_n, max_rm_n), "rows": rows})
                else:
                    dups.append({"tmdb": tid, "keep": rows[0], "remove": rows[1:]})
            else:
                review.append({"tmdb": tid, "why": "剧集季/集基本不重叠,疑似不同季(互补)", "rows": rows})
        else:
            dups.append({"tmdb": tid, "keep": rows[0], "remove": rows[1:]})
    return {"dups": dups, "review": review}


# ===== 删除 / 去重执行 =====
def _del_folder(lib, folder):
    L = fetch_libs(); fol = L.get(lib, {}).get("folder", lib)
    # path traversal guard:folder 必须在 CD/lib 和 STRM/lib 下,拒 ..、绝对路径、null 字节
    _safe_under(os.path.join(CD, fol), folder)
    _safe_under(os.path.join(STRM, fol), folder)
    done = []
    for base, label in [(os.path.join(CD, fol), "115"), (os.path.join(STRM, fol), "strm")]:
        p = os.path.join(base, folder)
        if os.path.isdir(p):
            shutil.rmtree(p); done.append(label)
    epost("/Library/Media/Updated", body={"Updates": [{"Path": "/strm/%s/%s" % (fol, folder), "UpdateType": "Deleted"}]})
    _undo_record("delete", {"lib": lib, "folder": folder, "deleted_from": done})
    return done


def exec_dedup(tmdb, removes):
    # 涉及多 lib,把所有相关 lib 锁一遍(顺序避免死锁)
    libs = sorted({r["lib"] for r in removes})
    locks = [_lib_lock(l) for l in libs]
    acquired = []
    try:
        for l, lk in zip(libs, locks):
            if not lk.acquire(blocking=False):
                raise AppError("库「%s」忙(扫描/移动/去重),稍后再试" % l, status=409)
            acquired.append(lk)
        res = []
        for r in removes:
            res.append({"folder": r["folder"], "deleted": _del_folder(r["lib"], r["folder"])})
        log("去重 tmdb %s 删 %d 个" % (tmdb, len(removes)))
        return {"tmdb": tmdb, "removed": res}
    finally:
        for lk in acquired: lk.release()


def delete_item(lib, folder, emby_id):
    done = _del_folder(lib, folder)
    if emby_id:
        edelete("/Items/%s" % emby_id)
    log("删除 [%s] %s" % (lib, folder))
    return {"deleted": done, "folder": folder}


# ===== 移动:跨库重命名 + 重建 strm =====
def move_item(from_lib, folder, to_lib, emby_id):
    L = fetch_libs()
    if from_lib not in L or to_lib not in L:
        return {"err": "未知库"}
    libs = sorted({from_lib, to_lib})
    locks = [_lib_lock(l) for l in libs]
    acquired = []
    try:
        for l, lk in zip(libs, locks):
            if not lk.acquire(blocking=False):
                raise AppError("库「%s」忙,稍后再试" % l, status=409)
            acquired.append(lk)
        return _move_item_locked(from_lib, folder, to_lib, emby_id, L)
    finally:
        for lk in acquired: lk.release()


def _move_item_locked(from_lib, folder, to_lib, emby_id, L):
    ff = L[from_lib]["folder"]; tf = L[to_lib]["folder"]
    # path traversal guard:src 和 dst 都必须在对应库下
    src = _safe_under(os.path.join(CD, ff), folder)
    dst = _safe_under(os.path.join(CD, tf), folder)
    if not os.path.isdir(src):
        return {"err": "源 115 文件夹不存在"}
    if os.path.exists(dst):
        return {"err": "目标已存在同名文件夹"}
    os.rename(src, dst)
    old_strm = os.path.join(STRM, ff, folder)
    if os.path.isdir(old_strm):
        shutil.rmtree(old_strm)
    media = "/media/" + tf; n = 0
    for root, ds, fs in os.walk(dst):
        rel = os.path.relpath(root, os.path.join(CD, tf))
        for f in sorted(fs):
            if f.lower().endswith(VE):
                dd = os.path.join(STRM, tf, rel); os.makedirs(dd, exist_ok=True)
                with open(os.path.join(dd, os.path.splitext(f)[0] + ".strm"), "w", encoding="utf-8") as w:
                    w.write(media + "/" + os.path.join(rel, f))
                n += 1
    epost("/Library/Media/Updated", body={"Updates": [{"Path": "/strm/%s/%s" % (ff, folder), "UpdateType": "Deleted"}]})
    if emby_id:
        edelete("/Items/%s" % emby_id)
    epost("/Items/%s/Refresh" % L[to_lib]["id"], {"Recursive": "true", "MetadataRefreshMode": "Default", "ImageRefreshMode": "Default"})
    log("移动 %s: %s -> %s" % (folder, from_lib, to_lib))
    _undo_record("move", {"from": from_lib, "to": to_lib, "folder": folder, "emby_id": emby_id, "strm_count": n})
    return {"ok": True, "moved": folder, "strm": n, "from": from_lib, "to": to_lib}


# ===== 库创建(沿用别的同 type 库的 LibraryOptions,避免开默认 RealtimeMonitor 等踩雷) =====
def create_library(name, ctype):
    name = (name or "").strip()
    if not name:
        raise AppError("库名不能为空", status=400)
    if ctype not in ("tvshows", "movies"):
        raise AppError("类型只能 tvshows/movies", status=400)
    if name in fetch_libs():
        raise AppError("已存在同名库", status=409)
    folder = name
    # path traversal guard:库名作为文件夹名,不能含 ../或绝对路径
    # 让 ValueError 直接冒到 do_POST 的 except ValueError → 400
    _safe_under(STRM, folder); _safe_under(CD, folder)
    os.makedirs(os.path.join(STRM, folder), exist_ok=True)
    os.makedirs(os.path.join(CD, folder), exist_ok=True)
    src_opts = {}
    try:
        for f in eget("/Library/VirtualFolders"):
            if f.get("CollectionType") == ctype and next((l for l in (f.get("Locations") or []) if l.startswith("/strm/")), None):
                src_opts = f.get("LibraryOptions") or {}; break
    except Exception:
        pass
    src_opts["PathInfos"] = [{"Path": "/strm/" + folder}]
    code = epost("/Library/VirtualFolders", {"name": name, "collectionType": ctype, "paths": "/strm/" + folder, "refreshLibrary": "false"},
                 {"LibraryOptions": src_opts})
    time.sleep(1)
    L = fetch_libs()
    if name in L:
        log("新建库 %s (%s) id=%s" % (name, ctype, L[name]["id"]))
        return {"ok": True, "name": name, "id": L[name]["id"]}
    return {"err": "创建后未在库列表找到 (HTTP %s)" % code}


# ===== 单库 list_items(给 UI 选项用) =====
def list_items(name):
    L = fetch_libs()
    if name not in L:
        return {"err": "未知库"}
    m = L[name]; typ = "Series" if m["ctype"] == "tvshows" else "Movie"
    items = eget("/Items", {"ParentId": m["id"], "Recursive": "true", "IncludeItemTypes": typ,
                            "Fields": "Path,ProductionYear,ProviderIds", "SortBy": "SortName", "Limit": 30000}).get("Items", [])
    out = []
    sep = "/" + m["folder"] + "/"
    for i in items:
        path = i.get("Path") or ""
        # 从 Path 抽 top-level folder:找 /<libfolder>/ 之后第一段
        folder = ""
        if sep in path:
            folder = path.split(sep, 1)[1].split("/", 1)[0]
        out.append({"id": i.get("Id"), "name": i.get("Name") or "(无名)", "year": i.get("ProductionYear"),
                    "tmdb": (i.get("ProviderIds") or {}).get("Tmdb", ""), "folder": folder})
    return {"lib": name, "items": out}


# ===== 追更剧集状态 =====
def zhuigeng_status():
    """查名字含「追更」的剧集库,标出哪些剧还在更新中(TMDb Status=Continuing)。"""
    out = []
    for name, m in fetch_libs().items():
        if "追更" not in name or m["ctype"] != "tvshows":
            continue
        try:
            series = eget("/Items", {"ParentId": m["id"], "Recursive": "true", "IncludeItemTypes": "Series",
                                    "Fields": "Status", "SortBy": "SortName"}).get("Items", [])
        except Exception:
            series = []
        for s in series:
            try:
                eps = eget("/Shows/%s/Episodes" % s["Id"], {"Fields": "PremiereDate,LocationType"}).get("Items", [])
            except Exception:
                eps = []
            have = [e for e in eps if e.get("LocationType") != "Virtual"]
            dates = sorted([(e.get("PremiereDate") or "")[:10] for e in have if e.get("PremiereDate")])
            st = s.get("Status") or "?"
            out.append({"lib": name, "name": s["Name"], "status": st,
                        "airing": st in ("Continuing", "Returning Series"),
                        "count": len(have), "latest": dates[-1] if dates else ""})
    out.sort(key=lambda x: (not x["airing"], x["count"]))
    return {"items": out}


def series_gaps(series_id):
    """查一部剧缺哪些集:内部跳号 + TMDb 已知但没下的(Emby 虚拟集,含落后最新的尾部)。"""
    eps = eget("/Shows/%s/Episodes" % series_id, {"Fields": "ParentIndexNumber,IndexNumber,LocationType", "Limit": 6000}).get("Items", [])
    have = collections.defaultdict(set); virt = collections.defaultdict(set); noidx = 0
    for e in eps:
        s = e.get("ParentIndexNumber"); n = e.get("IndexNumber")
        if n is None:
            noidx += 1; continue
        (virt if e.get("LocationType") == "Virtual" else have)[s].add(n)
    def compact(g):
        if not g: return []
        r = []; a = p = g[0]
        for x in g[1:]:
            if x == p + 1: p = x
            else: r.append(str(a) if a == p else "%d-%d" % (a, p)); a = p = x
        r.append(str(a) if a == p else "%d-%d" % (a, p)); return r
    have_all = set(); virt_all = set()
    for s in have: have_all |= have[s]
    for s in virt: virt_all |= virt[s]
    # 绝对集号判定:正片季(季号>0)集号"全局不重复"(没两季都有 E1)+ 最大集号大 → 海贼王这种连续绝对编号
    pos = [have[s] for s in have if isinstance(s, int) and s > 0 and have[s]]
    total_pos = sum(len(x) for x in pos); union_pos = set().union(*pos) if pos else set()
    absolute = bool(union_pos) and total_pos == len(union_pos) and max(union_pos) > 50
    if absolute:
        hi = max(union_pos | virt_all)
        miss = [x for x in range(min(union_pos), hi + 1) if x not in union_pos]
        return {"mode": "absolute", "have": len(union_pos), "max_ep": max(union_pos), "tmdb_max": hi,
                "gaps": len(miss), "gap_list": compact(miss), "noidx": noidx, "seasons": []}
    seas = []; th = tg = mx = 0; tmdb_max = 0
    for s in sorted(set(have) | set(virt), key=lambda x: (x is None, x)):
        full = sorted(have[s] | virt[s])
        if not full:
            continue
        lo, hi = full[0], full[-1]
        miss = [x for x in range(lo, hi + 1) if x not in have[s]]
        seas.append({"season": s, "count": len(have[s]), "lo": lo, "hi": hi, "gaps": compact(miss), "gapcount": len(miss)})
        th += len(have[s]); tg += len(miss); mx = max(mx, max(have[s]) if have[s] else 0); tmdb_max = max(tmdb_max, hi)
    return {"mode": "season", "have": th, "gaps": tg, "max_ep": mx, "tmdb_max": tmdb_max, "noidx": noidx, "seasons": seas}


# ===== 系统信息(给仪表盘) =====
def _gb(kb): return round(kb / 1024 / 1024, 1)


def system_info():
    mi = {}
    try:
        for line in open("/proc/meminfo"):
            k, _, v = line.partition(":"); mi[k.strip()] = int(v.strip().split()[0])
    except Exception:
        pass
    total = mi.get("MemTotal", 0); avail = mi.get("MemAvailable", 0); used = total - avail
    swt = mi.get("SwapTotal", 0); swf = mi.get("SwapFree", 0)
    try:
        st = os.statvfs("/volume1"); dtot = st.f_blocks * st.f_frsize; dfree = st.f_bavail * st.f_frsize
    except Exception:
        dtot = dfree = 0
    conts = []
    try:
        out = subprocess.run([DOCKER, "ps", "-a", "--format", "{{.Names}}\t{{.Status}}"], capture_output=True, text=True, timeout=15).stdout
        for ln in out.strip().splitlines():
            n, _, s = ln.partition("\t"); conts.append({"name": n, "status": s, "up": s.startswith("Up")})
    except Exception as e:
        conts = [{"name": "docker 读取失败", "status": str(e)[:60], "up": False}]
    procs = []
    try:
        out = subprocess.run(["ps", "aux", "--sort=-%mem"], capture_output=True, text=True, timeout=15).stdout
        for ln in out.splitlines()[1:9]:
            p = ln.split(None, 10)
            if len(p) >= 11:
                procs.append({"mem": p[3], "rss_mb": int(p[5]) // 1024, "cmd": p[10][:54]})
    except Exception:
        pass
    try:
        cd_ok = os.path.isdir(CD) and len(os.listdir(CD)) > 0
    except Exception:
        cd_ok = False
    return {"mem": {"total": _gb(total), "used": _gb(used), "avail": _gb(avail), "pct": round(used * 100 / total) if total else 0},
            "swap": {"total": _gb(swt), "used": _gb(swt - swf), "pct": round((swt - swf) * 100 / swt) if swt else 0},
            "disk": {"total_tb": round(dtot / 1e12, 2), "free_tb": round(dfree / 1e12, 2), "pct": round((dtot - dfree) * 100 / dtot) if dtot else 0},
            "containers": conts, "procs": procs, "cd_ok": cd_ok}


# ===== 配置 get/set(get 自动 mask cookie) =====
def get_config():
    ck = CFG.get("c115_cookie", "")
    mask = (ck[:18] + "…" + ck[-18:]) if len(ck) > 50 else ck
    return {"emby_url": CFG["emby_url"], "api_key": CFG["api_key"], "port": CFG["port"],
            "c115_cookie_set": bool(ck), "c115_cookie_mask": mask,
            "c115_cid_map": CFG.get("c115_cid_map") or {}}


def set_config(b):
    # lazy import 避免 lib.config → lib.auth → lib.logger → ... 循环风险
    from lib.config import CFG_LOCK, WEAK_PWS, save_cfg
    from lib.auth import _hash_password, _verify_password
    changed = []
    with CFG_LOCK:
        if b.get("password"):
            pw = b["password"]
            old = b.get("old_password", "")
            cur_hash = CFG.get("password_hash", "")
            # grace:首次升级(无 last_password_change_at 字段)允许一次无 old_password 改密;
            # 之后必须输旧密码且匹配 hash
            if CFG.get("last_password_change_at") and not _verify_password(old, cur_hash):
                raise AppError("旧密码错误", status=403)
            if len(pw) < 6:
                return {"err": "密码至少 6 位"}
            if pw in WEAK_PWS:
                return {"err": "密码在弱密码列表,换一个"}
            CFG["password_hash"] = _hash_password(pw); CFG.pop("password", None)
            CFG["last_password_change_at"] = int(time.time())
            changed.append("登录密码")
        if b.get("emby_url"):
            CFG["emby_url"] = b["emby_url"].strip(); changed.append("Emby地址")
        if b.get("api_key"):
            CFG["api_key"] = b["api_key"].strip(); changed.append("API Key")
        if b.get("c115_cookie") is not None:
            CFG["c115_cookie"] = b["c115_cookie"].strip(); changed.append("115 Cookie")
        if isinstance(b.get("c115_cid_map"), dict):
            CFG["c115_cid_map"] = {k: str(v).strip() for k, v in b["c115_cid_map"].items() if str(v).strip()}
            changed.append("115 库 cid 映射")
        save_cfg()
    log("修改配置: " + "、".join(changed))
    return {"ok": True, "changed": changed, "emby": emby_online()}


# ===== 配置导出/导入(剔密) =====
SENSITIVE_KEYS = ("password_hash", "c115_cookie")
# PROTECTED_IMPORT_KEYS:import 时**永远跳过**,无论用户传什么值。
# 包括 schema_version(不让绕 migration)、敏感字段(防直接覆盖植入)、
# **last_password_change_at(防 grace 复活提权)**、username(防越权)。
PROTECTED_IMPORT_KEYS = ("schema_version", "password_hash", "c115_cookie",
                         "last_password_change_at", "username")


def export_config():
    """返 redacted CFG —— 密码 hash 和 cookie raw 替换为 '<redacted>'(供用户下载备份)。"""
    from lib.config import CFG as _CFG
    out = {}
    for k, v in _CFG.items():
        if k in SENSITIVE_KEYS and v:
            out[k] = "<redacted>"
        else:
            out[k] = v
    return out


def import_config(b):
    """导入 config(必须 confirm=true)。schema 不匹配拒绝;PROTECTED_IMPORT_KEYS 永远不接受用户值。

    安全模型:
    - 敏感字段(password_hash / c115_cookie)即使非 <redacted> 也忽略,
      因为合法导出必为 <redacted>;非 <redacted> = 攻击者植入。
    - last_password_change_at 永远不接受导入 — 防止 import {last_password_change_at: null}
      复活 grace 模式 → 接着 set_config {password: ..., old_password: ""} 提权改密。
    - username / schema_version 同理不让动。
    """
    from lib.config import CFG as _CFG, CFG_LOCK, save_cfg, CURRENT_SCHEMA
    if not b.get("confirm"):
        raise AppError("必须显式 confirm=true", status=400)
    cfg = b.get("cfg") or {}
    if not isinstance(cfg, dict):
        raise AppError("cfg 必须是 dict", status=400)
    # schema 检查:旧导出包不能强压到新 schema 上
    sv = cfg.get("schema_version")
    if sv is not None and sv != CURRENT_SCHEMA:
        raise AppError("schema 不匹配:导入 %s vs 当前 %s" % (sv, CURRENT_SCHEMA), status=400)
    applied = []; skipped_protected = []
    with CFG_LOCK:
        for k, v in cfg.items():
            if k in PROTECTED_IMPORT_KEYS:
                skipped_protected.append(k)
                continue
            _CFG[k] = v
            applied.append(k)
        save_cfg()
    log("config 导入: 改 %d 字段 [%s]%s" % (
        len(applied), ", ".join(applied),
        " · 拒受保护字段 " + ",".join(skipped_protected) if skipped_protected else ""))
    return {"ok": True, "applied": applied, "skipped_protected": skipped_protected}


# ===== 异步任务:全库扫描 + c115 批处理 =====
def scan_all_async(tid):
    libs = list(fetch_libs().keys())
    task_set(tid, total=len(libs))
    out = []; tot_new = 0; tot_orph = 0; attn = []
    for i, name in enumerate(libs):
        if task_is_cancelled(tid): break
        task_set(tid, status_text="扫 " + name)
        try:
            r = scan_lib(name)
            tot_new += r.get("new_count", 0); tot_orph += r.get("orphans_cleaned", 0)
            for a in (r.get("attention") or []): attn.append(name + ": " + a)
        except Exception as e:
            r = {"err": str(e)}
        out.append({"lib": name, "result": r})
        task_set(tid, progress=i + 1)
    return {"libs_scanned": len(out), "new_count": tot_new, "orphans_cleaned": tot_orph, "attention": attn, "results": out}


def zhuigeng_status_async(tid):
    """zhuigeng_status 异步版:按库切分进度。逻辑内联(不调 zhuigeng_status)以便细粒度上报。"""
    libs = [n for n, m in fetch_libs().items() if "追更" in n and m["ctype"] == "tvshows"]
    task_set(tid, total=len(libs))
    out_items = []
    for i, name in enumerate(libs):
        if task_is_cancelled(tid): break
        task_set(tid, progress=i, status_text="查 " + name)
        m = fetch_libs().get(name)
        if not m: continue
        try:
            series = eget("/Items", {"ParentId": m["id"], "Recursive": "true", "IncludeItemTypes": "Series",
                                     "Fields": "Status", "SortBy": "SortName"}).get("Items", [])
        except Exception:
            series = []
        for s in series:
            if task_is_cancelled(tid): break
            try:
                eps = eget("/Shows/%s/Episodes" % s["Id"], {"Fields": "PremiereDate,LocationType"}).get("Items", [])
            except Exception:
                eps = []
            have = [e for e in eps if e.get("LocationType") != "Virtual"]
            dates = sorted([(e.get("PremiereDate") or "")[:10] for e in have if e.get("PremiereDate")])
            st = s.get("Status") or "?"
            out_items.append({"lib": name, "name": s["Name"], "status": st,
                              "airing": st in ("Continuing", "Returning Series"),
                              "count": len(have), "latest": dates[-1] if dates else "",
                              "tmdb": (s.get("ProviderIds") or {}).get("Tmdb", "")})
        task_set(tid, progress=i + 1)
    out_items.sort(key=lambda x: (not x["airing"], x["count"]))
    return {"items": out_items}


def list_strm(lib, folder):
    """列指定 lib 的 folder 下所有 strm 文件(给去重 review 区"查看文件列表"用)。
    路径越权 → ValueError(do_GET 转 400)。未知 lib → AppError(404)。"""
    L = fetch_libs()
    if lib not in L:
        raise AppError("未知库 %s" % lib, status=404)
    fol = L[lib]["folder"]
    base = _safe_under(os.path.join(STRM, fol), folder)
    if not os.path.isdir(base):
        return {"lib": lib, "folder": folder, "files": []}
    out = []
    for root, ds, fs in os.walk(base):
        for f in sorted(fs):
            if not f.endswith(".strm"):
                continue
            p = os.path.join(root, f)
            try:
                target = open(p, encoding="utf-8").read().strip()
            except Exception:
                target = "(读不到)"
            rel = os.path.relpath(p, base)
            out.append({"rel": rel, "target": target})
    return {"lib": lib, "folder": folder, "files": out}


def fix_poster_batch_async(tid, ids, typ):
    """批量自动修海报。保守:对每个无海报条目,取 remote_search 第一个 name 含原 folder 关键词且有 img 的候选。
    返 {results=[{id, name, ok, tmdb, err}], ok_count, total}"""
    task_set(tid, total=len(ids))
    results = []
    for i, item_id in enumerate(ids):
        if task_is_cancelled(tid): break
        task_set(tid, progress=i, status_text="修 " + str(item_id)[:8])
        try:
            it = eget("/Items", {"Ids": item_id, "Fields": "Name,Path"}).get("Items", [{}])[0]
            name = it.get("Name", "")
            # 用 path 倒数第二段作 folder 名(更接近文件夹原名),fallback 到 Name
            folder = (it.get("Path") or "").split("/")[-2] if it.get("Path") else name
            # 去掉年份等 (YYYY) / [xxx] / 【xxx】/(YYYY) 后缀(中英文括号都支持)
            search_name = re.sub(r'[(（\[【].*$', '', folder).strip() or name
            cands = remote_search(item_id, search_name, typ)
            picked = None
            for c in cands:
                if c.get("img") and search_name in (c.get("name") or ""):
                    picked = c; break
            if not picked:
                results.append({"id": item_id, "name": name, "ok": False, "err": "无合适候选"})
            else:
                r = apply_match(item_id, picked["tmdb"], typ, name)
                results.append({"id": item_id, "name": name, "ok": bool(r.get("poster")),
                                "tmdb": picked["tmdb"],
                                "err": "" if r.get("poster") else "已绑定但海报未到"})
        except Exception as e:
            results.append({"id": item_id, "name": "(?)", "ok": False, "err": str(e)})
        task_set(tid, progress=i + 1)
        time.sleep(0.5)  # 反 TMDb / Emby 频控
    ok_n = sum(1 for r in results if r["ok"])
    log("批量海报修复 → ✓ %d / 共 %d" % (ok_n, len(results)))
    return {"results": results, "ok_count": ok_n, "total": len(results)}


def delete_batch_async(tid, lib, items):
    """批量删除(items=[{folder, id}])。复用 delete_item 单条逻辑 + 进度上报。"""
    task_set(tid, total=len(items))
    results = []
    for i, it in enumerate(items):
        if task_is_cancelled(tid): break
        task_set(tid, progress=i, status_text="删 " + (it.get("folder") or "?")[:40])
        try:
            delete_item(lib, it.get("folder"), it.get("id"))
            results.append({"folder": it.get("folder"), "ok": True})
        except Exception as e:
            results.append({"folder": it.get("folder"), "ok": False, "err": str(e)})
        task_set(tid, progress=i + 1)
    ok_n = sum(1 for r in results if r["ok"])
    log("批量删除[%s] → ✓ %d / 共 %d" % (lib, ok_n, len(results)))
    return {"lib": lib, "results": results, "ok_count": ok_n, "total": len(results)}


def move_batch_async(tid, from_lib, to_lib, items):
    """批量移动(items=[{folder, id}])from_lib → to_lib。"""
    task_set(tid, total=len(items))
    results = []
    for i, it in enumerate(items):
        if task_is_cancelled(tid): break
        task_set(tid, progress=i, status_text="移 " + (it.get("folder") or "?")[:40])
        try:
            r = move_item(from_lib, it.get("folder"), to_lib, it.get("id"))
            if r.get("err"):
                results.append({"folder": it.get("folder"), "ok": False, "err": r["err"]})
            else:
                results.append({"folder": it.get("folder"), "ok": True})
        except Exception as e:
            results.append({"folder": it.get("folder"), "ok": False, "err": str(e)})
        task_set(tid, progress=i + 1)
    ok_n = sum(1 for r in results if r["ok"])
    log("批量移动 %s→%s → ✓ %d / 共 %d" % (from_lib, to_lib, ok_n, len(results)))
    return {"from": from_lib, "to": to_lib, "results": results, "ok_count": ok_n, "total": len(results)}


def dedup_exec_batch_async(tid, groups):
    """批量去重(groups=[{tmdb, remove:[{lib, folder}]}])聚合多组结果。"""
    task_set(tid, total=len(groups))
    results = []
    for i, g in enumerate(groups):
        if task_is_cancelled(tid): break
        task_set(tid, progress=i, status_text="去重 tmdb " + str(g.get("tmdb"))[:12])
        try:
            r = exec_dedup(g.get("tmdb"), g.get("remove", []))
            results.append({"tmdb": g.get("tmdb"), "ok": True, "removed": len(r.get("removed", []))})
        except Exception as e:
            results.append({"tmdb": g.get("tmdb"), "ok": False, "err": str(e)})
        task_set(tid, progress=i + 1)
    ok_n = sum(1 for r in results if r["ok"])
    return {"results": results, "ok_count": ok_n, "total": len(results)}


def replace_folder(lib, win_folder, lose_folder):
    """全替换:删 lose folder(115 → 回收站)+ 如果 win 是「lose(1)」格式,把 win 改名为 lose 名(保留 emby 路径)。
    场景:新分享 receive 后 115 自动加 (1) 后缀,新版完整旧版被废 → win=新(1) / lose=旧 → 删旧 + 新改名回原。
    要求:win 和 lose 必须在同库(115 跨库 rename 不可靠)。"""
    L = fetch_libs()
    if lib not in L:
        raise AppError("未知库 " + str(lib), status=404)
    fol = L[lib]["folder"]
    lose_cd = _safe_under(os.path.join(CD, fol), lose_folder)
    win_cd = _safe_under(os.path.join(CD, fol), win_folder)
    if not os.path.isdir(lose_cd):
        raise AppError("源 folder 不存在: " + lose_folder, status=404)
    if not os.path.isdir(win_cd):
        raise AppError("新 folder 不存在: " + win_folder, status=404)
    # 1. 删 lose (115 fuse → 回收站) + 同步删 strm
    shutil.rmtree(lose_cd)
    lose_strm = os.path.join(STRM, fol, lose_folder)
    if os.path.isdir(lose_strm):
        shutil.rmtree(lose_strm)
    # 2. 判断 win 是否「lose(1)」或「lose(N)」格式 — 是则改名回 lose
    renamed_to = win_folder
    import re as _re
    m = _re.match(r'^(.+?)(\(\d+\)|\(\d+\))$', win_folder)
    base = m.group(1) if m else win_folder
    if base == lose_folder:
        # win 改名回原名(去掉 (1))
        target_cd = os.path.join(CD, fol, lose_folder)
        os.rename(win_cd, target_cd)
        renamed_to = lose_folder
        # 同步处理 strm:rename win strm 文件夹 + 改 strm content 里的 folder 名
        win_strm = os.path.join(STRM, fol, win_folder)
        target_strm = os.path.join(STRM, fol, lose_folder)
        if os.path.isdir(win_strm):
            if os.path.exists(target_strm):
                shutil.rmtree(target_strm)
            os.rename(win_strm, target_strm)
            # strm 内容里 /media/<lib>/<win_folder>/... → /media/<lib>/<lose_folder>/...
            for root, _ds, fs in os.walk(target_strm):
                for f in fs:
                    if not f.endswith(".strm"):
                        continue
                    p = os.path.join(root, f)
                    try:
                        content = open(p, encoding="utf-8").read()
                        # 简单替换 first occurrence 即可(content 里 win_folder 通常只出现一次,在 /media/lib/folder/... 路径里)
                        new_content = content.replace("/" + win_folder + "/", "/" + lose_folder + "/", 1)
                        if new_content != content:
                            with open(p, "w", encoding="utf-8") as w:
                                w.write(new_content)
                    except Exception:
                        logger.exception("改 strm content 失败 %s", p)
    # 3. 通知 emby 重扫该 folder(路径变化)
    epost("/Library/Media/Updated", body={"Updates": [
        {"Path": "/strm/%s/%s" % (fol, lose_folder), "UpdateType": "Modified"}
    ]})
    log("替换 [%s] 用 %s 替掉 %s%s" % (lib, win_folder, lose_folder,
        " (并改名回原名)" if renamed_to == lose_folder else ""))
    _undo_record("replace", {"lib": lib, "win_was": win_folder, "lose_was": lose_folder,
                              "now_folder": renamed_to})
    return {"ok": True, "lib": lib, "kept_as": renamed_to,
            "dropped": lose_folder,
            "msg": "已替换:删了「%s」%s" % (lose_folder,
                "新 folder 改名回「%s」" % lose_folder if renamed_to == lose_folder else "")}


def add_new_pipeline_async(tid, items, default_lib, save_to_lib_fn):
    """一条龙加新资源 pipeline:批量 receive → scan 涉及库 → 等刮削 → 海报+重复检查 → 聚合 report。

    items: [{url, pwd, lib?}]  默认 lib = default_lib
    save_to_lib_fn: 注入 app 模块的 c115_save_to_lib(让 patch 链能贯穿测试 / 走 app._c115_req)
    返:{shares, libs_scanned, noposter, dups}
    """
    report = {"shares": [], "libs_scanned": {}, "noposter": [], "dups": [], "dups_review": []}
    affected_libs = set()
    # 总步数:N 个分享 + M 个库扫 + 1 刮削等待 + 1 海报 + 1 重复 — 但实时只知道前 N+1,后面边走边更
    task_set(tid, total=len(items) + 1, status_text="开始一条龙…")
    # Phase 1:逐个 receive
    for i, it in enumerate(items):
        if task_is_cancelled(tid): break
        url = (it.get("url") or "").strip()
        pwd = (it.get("pwd") or "").strip()
        lib = it.get("lib") or default_lib
        if not url:
            continue
        affected_libs.add(lib)
        task_set(tid, progress=i, status_text="转存 " + url[-40:])
        try:
            r = save_to_lib_fn(url, pwd, lib)
            report["shares"].append({
                "url": url, "lib": lib,
                "ok": bool(r.get("ok")),
                "title": r.get("title") or r.get("share") or url[-30:],
                "count": r.get("count", 0),
                "msg": r.get("msg") or r.get("err") or ""
            })
        except Exception as e:
            report["shares"].append({"url": url, "lib": lib, "ok": False, "title": url[-30:], "count": 0, "msg": str(e)})
        time.sleep(0.8)  # anti-115-rate-limit
    # 更新 total 现在能算了
    libs_list = sorted(affected_libs)
    new_total = len(items) + len(libs_list) + 3  # +刮削等待+海报+重复
    task_set(tid, total=new_total)
    # Phase 2:扫各涉及 lib
    for j, lib in enumerate(libs_list):
        if task_is_cancelled(tid): break
        task_set(tid, progress=len(items) + j, status_text="扫库 " + lib)
        try:
            r = scan_lib(lib)
            report["libs_scanned"][lib] = {
                "new_count": r.get("new_count", 0),
                "orphans_cleaned": r.get("orphans_cleaned", 0),
                "matched": r.get("matched", 0),
                "attention": r.get("attention") or [],
            }
        except Exception as e:
            report["libs_scanned"][lib] = {"err": str(e)}
    # Phase 3:等 Emby 刮削(给海报检查时间)
    if libs_list and not task_is_cancelled(tid):
        task_set(tid, progress=len(items) + len(libs_list), status_text="等 Emby 刮削 8s…")
        for _ in range(8):
            if task_is_cancelled(tid): break
            time.sleep(1)
    # Phase 4:海报检查(过滤涉及库)
    if not task_is_cancelled(tid):
        task_set(tid, progress=len(items) + len(libs_list) + 1, status_text="检查海报")
        try:
            np_all = list_noposter()
            report["noposter"] = [x for x in np_all if x.get("lib") in affected_libs]
        except Exception as e:
            report["noposter_err"] = str(e)
    # Phase 5:重复检查(只标涉及库的新 dup)
    if not task_is_cancelled(tid):
        task_set(tid, progress=len(items) + len(libs_list) + 2, status_text="检查重复")
        try:
            d = analyze_dups()
            dups = d.get("dups") or []
            review = d.get("review") or []
            # 过滤:只显示涉及库的(keep 或 remove 任一在 affected_libs)
            def involves(group):
                rows = group.get("rows") or ([group.get("keep")] if group.get("keep") else []) + (group.get("remove") or [])
                return any((r and r.get("lib") in affected_libs) for r in rows)
            report["dups"] = [g for g in dups if involves(g)][:30]
            report["dups_review"] = [g for g in review if involves(g)][:30]
        except Exception as e:
            report["dups_err"] = str(e)
    task_set(tid, progress=new_total, status_text="完成")
    ok_n = sum(1 for s in report["shares"] if s["ok"])
    log("一条龙: 转存 ✓%d/共%d → 扫 %d 个库 → 无海报 %d → 重复 %d 组" % (
        ok_n, len(report["shares"]), len(libs_list),
        len(report["noposter"]), len(report["dups"]) + len(report["dups_review"])))
    return report


def scan_lib_async(tid, name, keyword=None):
    """scan_lib 的异步包装:进度通过 task_set,扫到一半可取消(checkpoint 在文件夹间)。"""
    L = fetch_libs()
    if name not in L:
        return {"err": "未知库 " + str(name)}
    lock = _lib_lock(name)
    if not lock.acquire(blocking=False):
        raise AppError("库「%s」已有扫描在跑" % name, status=409)
    try:
        meta = L[name]; folder = meta["folder"]
        src_base = os.path.join(CD, folder); strm_base = os.path.join(STRM, folder); media = "/media/" + folder
        if not os.path.isdir(src_base):
            return {"err": "115 文件夹不存在: " + src_base}
        kw = (keyword or "").strip()
        # 第一趟:列所有 top folder(用于设置 total 让前端进度条有意义)
        tops = sorted(t for t in os.listdir(src_base) if os.path.isdir(os.path.join(src_base, t)) and (not kw or kw in t))
        task_set(tid, total=len(tops), status_text="扫 " + name)
        new_files = []; new_folders = {}; attention = []; matched = 0
        for idx, top in enumerate(tops):
            if task_is_cancelled(tid):
                task_set(tid, status_text="取消中…"); break
            task_set(tid, progress=idx, status_text="扫 %s · %s" % (name, top[:40]))
            tp = os.path.join(src_base, top)
            matched += 1
            missing = []
            for root, ds, fs in os.walk(tp):
                rel = os.path.relpath(root, src_base)
                for f in sorted(fs):
                    if f.lower().endswith(VE):
                        sp = os.path.join(strm_base, rel, os.path.splitext(f)[0] + ".strm")
                        if not os.path.exists(sp):
                            missing.append((rel, f))
            if not missing:
                continue
            if re.search(r'tmdbid[-_]\d+', top, re.IGNORECASE) or os.path.isdir(os.path.join(strm_base, top)):
                for rel, f in missing:
                    dd = os.path.join(strm_base, rel); os.makedirs(dd, exist_ok=True)
                    with open(os.path.join(dd, os.path.splitext(f)[0] + ".strm"), "w", encoding="utf-8") as w:
                        w.write(media + "/" + os.path.join(rel, f))
                    new_files.append(f)
                new_folders[top] = len(missing)
            else:
                attention.append("%s (+%d个视频,无tmdbid且首次出现,需看一眼)" % (top, len(missing)))
        # 清孤儿:用 os.scandir 优化,先列顶层子目录,关键词过滤后只 walk 命中的(对大库 IO 友好)
        orphans = 0
        task_set(tid, status_text="清孤儿 strm…")
        if os.path.isdir(strm_base):
            for top_entry in os.scandir(strm_base):
                if task_is_cancelled(tid): break
                if not top_entry.is_dir(): continue
                if kw and kw not in top_entry.name: continue
                for root, ds, fs in os.walk(top_entry.path):
                    for f in fs:
                        if not f.endswith(".strm"): continue
                        p = os.path.join(root, f)
                        try: content = open(p, encoding="utf-8").read().strip()
                        except Exception: continue
                        if content.startswith("/media/"):
                            target = CD + content[len("/media"):]
                            if not os.path.exists(target):
                                os.remove(p); orphans += 1
        task_set(tid, progress=len(tops))
        if new_files or orphans:
            epost("/Items/%s/Refresh" % meta["id"], {"Recursive": "true", "MetadataRefreshMode": "Default", "ImageRefreshMode": "Default"})
            log("扫描[%s] async 新增 strm %d, 清孤儿 %d" % (name, len(new_files), orphans))
        return {"lib": name, "keyword": kw, "matched": matched, "new_count": len(new_files),
                "new_folders": new_folders, "attention": attention, "orphans_cleaned": orphans,
                "refreshed": bool(new_files or orphans)}
    finally:
        lock.release()


def c115_snap_batch_async(tid, c115_snap_full_fn, items, default_pwd=""):
    """c115_snap_full_fn 由 app 层注入(app.c115_snap_full),内部用 app._c115_req 受 patch 影响。"""
    task_set(tid, total=len(items))
    out = []
    for i, raw in enumerate(items):
        if task_is_cancelled(tid): break
        url = (raw.get("url") or "").strip()
        if not url: task_set(tid, progress=i + 1); continue
        pwd = (raw.get("pwd") or "").strip() or default_pwd
        task_set(tid, status_text="预览 " + url[:48])
        try: r = c115_snap_full_fn(url, pwd)
        except Exception as e: r = {"ok": False, "err": str(e)}
        out.append({"url": url, "ok": bool(r.get("ok")), "share": r.get("share"),
                    "title": r.get("share_title"), "size": r.get("file_size") or 0,
                    "count": len(r.get("files") or []), "err": r.get("err"),
                    "files_preview": (r.get("files") or [])[:5]})
        task_set(tid, progress=i + 1)
        time.sleep(0.8)
    return {"items": out, "total": len(out), "ok_count": sum(1 for x in out if x["ok"])}


def c115_save_batch_async(tid, c115_save_to_lib_fn, items, lib, default_pwd=""):
    task_set(tid, total=len(items))
    results = []; ok_n = 0
    for i, raw in enumerate(items):
        if task_is_cancelled(tid): break
        url = (raw.get("url") or "").strip()
        if not url: task_set(tid, progress=i + 1); continue
        pwd = (raw.get("pwd") or "").strip() or default_pwd
        task_set(tid, status_text="转存 " + url[:48])
        try: r = c115_save_to_lib_fn(url, pwd, lib)
        except Exception as e: r = {"ok": False, "err": str(e)}
        results.append({"url": url, "share": r.get("share"), "title": r.get("title"),
                        "ok": bool(r.get("ok")), "msg": r.get("msg") or r.get("err") or "",
                        "count": r.get("count", 0)})
        if r.get("ok"): ok_n += 1
        task_set(tid, progress=i + 1)
        time.sleep(1.0)
    log("115 批量转存(async) → 库「%s」: ✓ %d / 共 %d" % (lib, ok_n, len(results)))
    return {"ok": ok_n > 0, "lib": lib, "results": results, "ok_count": ok_n, "total": len(results)}


def c115_snap_batch(c115_snap_full_fn, items, default_pwd=""):
    out = []
    for raw in items:
        url = (raw.get("url") or "").strip()
        if not url: continue
        pwd = (raw.get("pwd") or "").strip() or default_pwd
        try: r = c115_snap_full_fn(url, pwd)
        except Exception as e: r = {"ok": False, "err": str(e)}
        out.append({"url": url, "ok": bool(r.get("ok")),
                    "share": r.get("share"), "title": r.get("share_title"),
                    "size": r.get("file_size") or 0,
                    "count": len(r.get("files") or []),
                    "err": r.get("err"),
                    "files_preview": (r.get("files") or [])[:5]})
        time.sleep(0.8)
    ok_n = sum(1 for x in out if x["ok"])
    return {"items": out, "total": len(out), "ok_count": ok_n}


def c115_save_batch(c115_save_to_lib_fn, items, lib, default_pwd=""):
    results = []; ok_n = 0
    for raw in items:
        url = (raw.get("url") or "").strip()
        if not url: continue
        pwd = (raw.get("pwd") or "").strip() or default_pwd
        try: r = c115_save_to_lib_fn(url, pwd, lib)
        except Exception as e: r = {"ok": False, "err": str(e)}
        results.append({"url": url, "share": r.get("share"), "title": r.get("title"),
                        "ok": bool(r.get("ok")),
                        "msg": r.get("msg") or r.get("err") or "",
                        "count": r.get("count", 0)})
        if r.get("ok"): ok_n += 1
        time.sleep(1.0)  # 115 anti-bot 缓冲
    log("115 批量转存 → 库「%s」: ✓ %d / 共 %d" % (lib, ok_n, len(results)))
    return {"ok": ok_n > 0, "lib": lib, "results": results, "ok_count": ok_n, "total": len(results)}
