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


def _c115_uid():
    m = re.search(r'UID=([^;\s]+)', CFG.get("c115_cookie", "") or "")
    return m.group(1).split("_")[0] if m else ""


def _c115_uid_for(cookie):
    """从任意 cookie 字符串解 UID(不依赖 CFG,test_candidate 用)。"""
    m = re.search(r'UID=([^;\s]+)', cookie or "")
    return m.group(1).split("_")[0] if m else ""


def _c115_req(path, params=None, post=None):
    """实际发请求(给 app.py 复用为 app._c115_req)。出错统一返 {state:False, error:...} 不抛。"""
    url = C115_API + path
    if params:
        url += "?" + urllib.parse.urlencode(params)
    headers = {"User-Agent": C115_UA, "Cookie": CFG.get("c115_cookie", "") or "",
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
    如果 cookie_override 提供 → 临时替换 CFG['c115_cookie'](req_fn 内部读 CFG),完事还原。
    **swap+restore 必须串行**(CFG_LOCK)— 否则两个并发 test_candidate 会相互污染真 cookie(P0-1 regression)。
    返 {ok, uid, used} 或 {ok:False, err}。"""
    cookie = cookie_override if cookie_override is not None else CFG.get("c115_cookie")
    if not cookie:
        return {"ok": False, "err": "未设置 cookie(去设置页填)"}
    if cookie_override is not None:
        # lazy import 避免循环(config 不依赖 c115)
        from lib.config import CFG_LOCK
        # 整段 swap → req → restore 必须串行;两个并发 candidate test 不能交错
        with CFG_LOCK:
            old = CFG.get("c115_cookie", "")
            CFG["c115_cookie"] = cookie_override
            try:
                r = req_fn("/files/index_info")
            finally:
                CFG["c115_cookie"] = old
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


def c115_snap(req_fn, share_code, receive_code):
    return req_fn("/share/snap", params={
        "share_code": share_code, "receive_code": receive_code or "",
        "cid": 0, "offset": 0, "limit": 1000,
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
    items = d.get("list") or []
    info = d.get("shareinfo") or {}
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


def c115_save_to_lib(req_fn, url, pwd, lib, file_ids=None):
    """单链接转存到指定 emby 库对应的 115 cid。"""
    s = c115_snap_full(req_fn, url, pwd)
    if not s.get("ok"): return s
    cid_map = CFG.get("c115_cid_map") or {}
    cid = cid_map.get(lib)
    if not cid: return {"ok": False, "err": "库「%s」没配 115 cid,先去设置页填(从 CloudDrive2 web 进入该目录,URL 末尾的数字就是 cid)" % lib}
    if not file_ids:
        file_ids = [f["id"] for f in s["files"] if f.get("id")]
    if not file_ids: return {"ok": False, "err": "分享内无可转存文件(snap 返回里没识别出 id,raw_sample=%s)" % json.dumps(s.get("_raw_sample"), ensure_ascii=False)[:300]}
    res = c115_receive_api(req_fn, s["share"], s["rc"], file_ids, cid)
    ok = bool(res.get("state"))
    log("115 转存 %s (%d项) → 库「%s」 cid=%s: %s" % (s["share"], len(file_ids), lib, cid, "✓" if ok else (res.get("error") or res.get("msg"))))
    return {"ok": ok, "share": s["share"], "count": len(file_ids), "lib": lib, "cid": cid,
            "title": s.get("share_title"),
            "msg": (res.get("error") or res.get("msg")) if not ok else ("已转存 %d 项到库「%s」" % (len(file_ids), lib))}
