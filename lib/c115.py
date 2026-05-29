"""115 网盘转存:webapi.115.com 网页接口 + cookie 鉴权。
**HTTP 函数 _c115_req 注入式** —— c115_snap_full/c115_list_dirs 第一参数收 req_fn,
方便 app.py 用 `_c115.c115_list_dirs(_c115_req, cid)` 包装,测试 `patch.object(app, "_c115_req", ...)`
就能命中实际调用点(避免跨模块 mock 失效的坑)。

纯函数(c115_parse_url)直接 import 用即可。
"""
import json, re, urllib.error, urllib.parse, urllib.request

from lib.config import CFG
from lib.logger import log


C115_UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36"
C115_API = "https://webapi.115.com"
C115_SITE = "https://115.com"   # 离线下载用主站域名(转存/snap 用 webapi)


def _c115_uid():
    m = re.search(r'UID=([^;\s]+)', CFG.get("c115_cookie", "") or "")
    return m.group(1).split("_")[0] if m else ""


def _c115_uid_for(cookie):
    """从任意 cookie 字符串解 UID(不依赖 CFG,test_candidate 用)。"""
    m = re.search(r'UID=([^;\s]+)', cookie or "")
    return m.group(1).split("_")[0] if m else ""


def _c115_req(path, params=None, post=None, cookie=None, host=None):
    """实际发请求(给 app.py 复用为 app._c115_req)。出错统一返 {state:False, error:...} 不抛。
    cookie=None 时回落全局 CFG['c115_cookie'];host=None 默认 webapi(转存/snap),离线下载传 C115_SITE。"""
    url = (host or C115_API) + path
    if params:
        url += "?" + urllib.parse.urlencode(params)
    use_cookie = cookie if cookie is not None else (CFG.get("c115_cookie", "") or "")
    headers = {"User-Agent": C115_UA, "Cookie": use_cookie,
               "Referer": "https://115.com/", "Accept": "application/json, text/plain, */*"}
    data = None
    if post is not None:
        data = urllib.parse.urlencode(post).encode("utf-8")
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    req = urllib.request.Request(url, data=data, headers=headers, method="POST" if data is not None else "GET")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read().decode("utf-8", "ignore")
            try: return json.loads(raw)
            except Exception: return {"state": False, "error": "非 JSON 响应: " + raw[:200]}
    except urllib.error.HTTPError as e:
        body = ""
        try: body = e.read().decode("utf-8", "ignore")[:200]
        except Exception: pass
        return {"state": False, "error": "HTTP %d: %s" % (e.code, body)}
    except Exception as e:
        return {"state": False, "error": str(e)}


def c115_test(req_fn, cookie_override=None):
    """检测 cookie 是否有效。
    cookie_override 给定时把它作为 cookie 参数透传 req_fn —— 不碰全局 CFG,
    所以并发的批量 115 请求不会被污染(取代旧的 swap-global 方案)。
    返 {ok, uid, used} 或 {ok:False, err}。"""
    cookie = cookie_override if cookie_override is not None else CFG.get("c115_cookie")
    if not cookie:
        return {"ok": False, "err": "未设置 cookie(去设置页填)"}
    if cookie_override is not None:
        r = req_fn("/files/index_info", cookie=cookie_override)
        uid = _c115_uid_for(cookie_override)
    else:
        r = req_fn("/files/index_info")
        uid = _c115_uid()
    if r.get("state"):
        d = r.get("data") or {}
        si = (d.get("space_info") or {}).get("all_total") or (d.get("space_info") or {}).get("all_use") or {}
        return {"ok": True, "uid": uid, "used": si.get("size_format", "")}
    return {"ok": False, "err": r.get("error") or r.get("msg") or str(r)[:200]}


def c115_parse_url(url, pwd=None):
    """纯函数:url → (share_code, receive_code)。None/空/乱码全部 graceful 返 (None, None)/部分匹配。"""
    url = (url or "").strip()
    share = None; rc = None
    m = re.search(r'/s/([0-9a-zA-Z]+)', url)
    if m: share = m.group(1)
    m = re.search(r'[?&](?:password|pwd|pickcode)=([^&#\s]+)', url)
    if m: rc = m.group(1)
    if not share:
        parts = re.split(r'[\s,]+', url)
        if parts and re.match(r'^[0-9a-zA-Z]+$', parts[0]):
            share = parts[0]
            if len(parts) > 1 and re.match(r'^[0-9a-zA-Z]+$', parts[1]): rc = parts[1]
    if pwd: rc = pwd.strip()
    return share, rc


def c115_snap(req_fn, share_code, receive_code, offset=0, limit=1000):
    return req_fn("/share/snap", params={
        "share_code": share_code, "receive_code": receive_code or "",
        "cid": 0, "offset": offset, "limit": limit,
    })


def c115_receive_api(req_fn, share_code, receive_code, file_ids, target_cid):
    return req_fn("/share/receive", post={
        "share_code": share_code, "receive_code": receive_code or "",
        "file_id": ",".join(str(x) for x in file_ids),
        "cid": str(target_cid), "user_id": _c115_uid(),
    })


def c115_snap_full(req_fn, url, pwd):
    """解析分享链接 → snap → 返完整 {ok, share, rc, share_title, file_size, files, _raw_sample}。"""
    share, rc = c115_parse_url(url, pwd)
    if not share: return {"ok": False, "err": "解析不到 share_code(贴完整 115 分享链接,或 share_code+空格+提取码)"}
    snap = c115_snap(req_fn, share, rc)
    if not snap.get("state"):
        return {"ok": False, "err": snap.get("error") or snap.get("msg") or "snap 失败", "share": share, "rc": rc}
    d = snap.get("data") or {}
    items = list(d.get("list") or [])
    info = d.get("shareinfo") or {}
    # 分页:根层文件数 > 1000 时 snap 只返第一页 → 整季打包/合集会静默缺集。
    # 判据用「上一页是否取满 limit」而非依赖 shareinfo 里某个猜测的 total 字段(那字段名不确定,
    # 之前用 file_count 在多数响应里取不到 → 分页形同虚设)。每页 sleep 0.5s 保持风控友好,封顶 20 页。
    LIMIT = 1000
    last_chunk = len(d.get("list") or [])
    page = 1
    import time as _t
    while last_chunk >= LIMIT and page < 20:
        _t.sleep(0.5)
        nxt = c115_snap(req_fn, share, rc, offset=len(items), limit=LIMIT)
        if not nxt.get("state"): break
        chunk = nxt.get("data", {}).get("list") or []
        if not chunk: break
        items.extend(chunk); last_chunk = len(chunk); page += 1
    files = []
    for it in items:
        # 115 share/snap:文件用 fid;文件夹无 fid,自身 id 在 cid
        fid = it.get("fid") or it.get("file_id")
        is_dir = 0 if fid else 1
        item_id = fid or it.get("cid")  # 转存时 receive 的 file_id 字段:文件传 fid,文件夹传 cid
        files.append({"id": str(item_id) if item_id is not None else None,
                      "name": it.get("n") or it.get("name"),
                      "size": it.get("s") or it.get("size") or 0,
                      "is_dir": is_dir})
    return {"ok": True, "share": share, "rc": rc,
            "share_title": info.get("share_title") or info.get("file_name"),
            "file_size": info.get("file_size") or info.get("total_size"),
            "files": files,
            "_raw_sample": items[0] if items else None}


def c115_list_dirs(req_fn, cid="0"):
    """列 cid 目录下的子文件夹(只 folder,不要 file),返回 {ok, items:[{cid, name}], err?}。"""
    r = req_fn("/files", params={
        "aid": "1", "cid": str(cid), "o": "user_ptime", "asc": "0",
        "offset": 0, "limit": 1000, "show_dir": 1, "format": "json",
    })
    if not r.get("state"):
        return {"ok": False, "err": r.get("error") or r.get("msg") or "list 失败", "items": []}
    out = []
    for it in r.get("data") or []:
        # 115 约定:文件夹有 cid+pid 无 fid;文件有 fid
        if it.get("fid"):
            continue
        sub_cid = it.get("cid")
        if sub_cid is None:
            continue
        out.append({"cid": str(sub_cid), "name": it.get("n") or it.get("name", "")})
    return {"ok": True, "items": out}


def c115_auto_cid(req_fn, fetch_libs_fn, max_depth=2):
    """递归扫 115 目录(默认前 2 层),搜各 emby 库 folder 名对应的 115 cid。"""
    if not CFG.get("c115_cookie"):
        return {"ok": False, "err": "未设置 cookie"}
    libs = fetch_libs_fn()
    # folder名 → emby库显示名(用于回填 c115_cid_map 时按显示名 key)
    targets = {m["folder"]: name for name, m in libs.items()}
    matches = {}  # 库显示名 → [{cid, path}]
    visited = set(); walked = [0]
    def walk(cid, prefix, depth):
        if depth < 0 or cid in visited or walked[0] > 80:
            return
        visited.add(cid); walked[0] += 1
        r = c115_list_dirs(req_fn, cid)
        if not r.get("ok"):
            return
        for f in r["items"]:
            n = f["name"]; path = prefix + "/" + n
            if n in targets:
                matches.setdefault(targets[n], []).append({"cid": f["cid"], "path": path})
            walk(f["cid"], path, depth - 1)
    walk("0", "", max_depth)
    return {"ok": True, "matches": matches, "current": CFG.get("c115_cid_map") or {},
            "scanned": walked[0]}


def c115_save_to_cid(req_fn, url, pwd, cid, label=None, file_ids=None):
    """单链接转存到指定 115 cid(任意目录,不限 emby 库)。label 仅用于日志/返回文案。"""
    if not cid:
        return {"ok": False, "err": "未指定目标 115 目录 cid"}
    s = c115_snap_full(req_fn, url, pwd)
    if not s.get("ok"): return s
    if not file_ids:
        file_ids = [f["id"] for f in s["files"] if f.get("id")]
    if not file_ids: return {"ok": False, "err": "分享内无可转存文件(snap 返回里没识别出 id,raw_sample=%s)" % json.dumps(s.get("_raw_sample"), ensure_ascii=False)[:300]}
    res = c115_receive_api(req_fn, s["share"], s["rc"], file_ids, cid)
    ok = bool(res.get("state"))
    where = ("库「%s」" % label) if label else ("目录 cid=%s" % cid)
    log("115 转存 %s (%d项) → %s cid=%s: %s" % (s["share"], len(file_ids), where, cid, "✓" if ok else (res.get("error") or res.get("msg"))))
    return {"ok": ok, "share": s["share"], "count": len(file_ids), "cid": cid, "lib": label,
            "title": s.get("share_title"),
            "msg": (res.get("error") or res.get("msg")) if not ok else ("已转存 %d 项到%s" % (len(file_ids), where))}


def c115_save_to_lib(req_fn, url, pwd, lib, file_ids=None):
    """单链接转存到指定 emby 库对应的 115 cid(解析 lib→cid 后走 c115_save_to_cid)。"""
    cid = (CFG.get("c115_cid_map") or {}).get(lib)
    if not cid: return {"ok": False, "err": "库「%s」没配 115 cid,先去设置页填(从 CloudDrive2 web 进入该目录,URL 末尾的数字就是 cid)" % lib}
    return c115_save_to_cid(req_fn, url, pwd, cid, label=lib, file_ids=file_ids)


# ===== 115 离线下载(magnet / ed2k / http 直链)=====
def c115_offline_space(req_fn):
    """拿离线下载的 sign+time(add_task 必须)+ 配额。只读。"""
    return req_fn("/", params={"ct": "offline", "ac": "space"}, host=C115_SITE)


def c115_offline_add(req_fn, url, cid, label=None):
    """把 magnet/ed2k/http 直链加进 115 离线下载队列,下载到目标 cid。
    流程:先拿 sign/time,再 add_task_url。返 {ok, info_hash?, msg, quota?}。"""
    url = (url or "").strip()
    if not url:
        return {"ok": False, "err": "空链接"}
    if not cid:
        return {"ok": False, "err": "未指定目标 115 目录 cid"}
    sp = c115_offline_space(req_fn)
    if not sp.get("state"):
        return {"ok": False, "err": "拿离线 sign 失败(cookie 失效或无离线权限): " + str(sp.get("error") or sp.get("_err") or sp)[:120]}
    sign, t = sp.get("sign"), sp.get("time")
    # sign/time 必须非空才发真请求 —— 否则会带 sign=None 发出一次注定失败的真离线写请求(review)
    if not sign or not t:
        return {"ok": False, "err": "离线 sign/time 缺失(115 响应结构异常,可能接口变更): " + str(sp)[:120]}
    res = req_fn("/web/lixian/", params={"ct": "lixian", "ac": "add_task_url"},
                 post={"url": url, "wp_path_id": str(cid), "sign": sign, "time": t}, host=C115_SITE)
    ok = bool(res.get("state"))
    where = ("库「%s」" % label) if label else ("目录 cid=%s" % cid)
    log("115 离线 %s → %s: %s" % (url[:50], where, "✓ " + str(res.get("info_hash", "")) if ok else (res.get("error_msg") or res.get("error") or res.get("errtype") or res)))
    if ok:
        return {"ok": True, "info_hash": res.get("info_hash"), "cid": cid, "lib": label,
                "msg": "已加入 115 离线下载队列(到 115 看进度)→ %s" % where}
    # 错误兜底:errcode 缺失时不显示无意义的「errcode=None」,回带原始响应片段
    ec = res.get("errcode")
    err = res.get("error_msg") or res.get("error") or res.get("errtype") or \
          (("离线添加失败(errcode=%s)" % ec) if ec is not None else ("离线添加失败: " + str(res)[:120]))
    return {"ok": False, "info_hash": res.get("info_hash"), "err": err}
