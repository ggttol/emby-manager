"""Emby HTTP API 薄封装 + 库枚举 + 用户管理。
所有调用走 CFG["emby_url"] + CFG["api_key"],错误回 HTTP code 不抛(epost/edelete 吞 HTTPError)。
"""
import json, urllib.error, urllib.parse, urllib.request

from lib.config import CFG
from lib.logger import logger


def _url(path, params=None):
    p = dict(params or {}); p["api_key"] = CFG["api_key"]
    # path 里可能插了来自请求的 item id(/Items/<id>/Refresh 等)。percent-encode 整段 path(保留 /),
    # 这样 id 里若带 ?/&/# 会被编码成字面量,无法越出成查询参数注入(review:emby id 注入)。
    # 合法 id(hex GUID)不含这些字符,编码后不变。
    safe_path = urllib.parse.quote(path, safe="/%:@")
    return CFG["emby_url"] + safe_path + "?" + urllib.parse.urlencode(p)


def eget(path, params=None):
    with urllib.request.urlopen(_url(path, params), timeout=60) as r:
        return json.loads(r.read())


def epost(path, params=None, body=None):
    data = json.dumps(body).encode() if body is not None else None
    h = {"Content-Type": "application/json"} if body is not None else {}
    req = urllib.request.Request(_url(path, params), data=data, method="POST", headers=h)
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.getcode()
    except urllib.error.HTTPError as e:
        return e.code


def edelete(path):
    try:
        with urllib.request.urlopen(urllib.request.Request(_url(path), method="DELETE"), timeout=60) as r:
            return r.getcode()
    except urllib.error.HTTPError as e:
        return e.code


def emby_online():
    try:
        info = eget("/System/Info/Public")
        return {"online": True, "name": info.get("ServerName"), "version": info.get("Version")}
    except Exception as e:
        return {"online": False, "err": str(e)}


def lib_count(pid, typ):
    try:
        return eget("/Items", {"ParentId": pid, "Recursive": "true", "IncludeItemTypes": typ, "Limit": 0}).get("TotalRecordCount", 0)
    except Exception:
        return 0


def fetch_libs():
    """动态从 Emby 读 strm 库,只回兼容的 {name:{id,ctype,folder}}。"""
    return fetch_libs_full()[0]


def fetch_libs_full():
    """返回 (included, excluded[{name,reason}]):UI 能展示为啥某库被忽略。"""
    out = {}; excluded = []
    try:
        vf = eget("/Library/VirtualFolders")
    except Exception as e:
        logger.warning("读 VirtualFolders 失败: %s", e)
        return {}, [{"name": "(读取失败)", "reason": str(e)}]
    for f in vf:
        name = f.get("Name") or "(无名)"
        locs = f.get("Locations") or []
        strm_loc = next((l for l in locs if l.startswith("/strm/")), None)
        if not strm_loc:
            excluded.append({"name": name, "reason": "无 /strm/ 路径(boxset 或别的库类型)"})
            continue
        folder = strm_loc.rstrip("/").split("/strm/", 1)[1].split("/")[0]
        if not folder:
            excluded.append({"name": name, "reason": "/strm/ 路径解析空 folder"})
            continue
        out[name] = {"id": f.get("ItemId"), "ctype": f.get("CollectionType") or "mixed", "folder": folder}
    return out, excluded


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


def remote_search(item_id, name, typ):
    body = {"SearchInfo": {"Name": name, "ProviderIds": {}}, "ItemId": item_id, "IncludeDisabledProviders": True}
    kind = "Series" if typ == "Series" else "Movie"
    try:
        req = urllib.request.Request(_url("/Items/RemoteSearch/" + kind), data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"}, method="POST")
        with urllib.request.urlopen(req, timeout=60) as r:
            res = json.loads(r.read())
        return [{"name": c.get("Name"), "year": c.get("ProductionYear"), "tmdb": str(c.get("ProviderIds", {}).get("Tmdb") or ""),
                 "img": c.get("ImageUrl") or "", "overview": (c.get("Overview") or "")[:160]} for c in res[:8]]
    except Exception:
        return []


def apply_match(item_id, tmdb, typ, name):
    import time, re
    from lib.logger import log, AppError
    # tmdb 必须纯数字 —— 否则会被原样写进 Emby ProviderIds.Tmdb,再在前端列表里拼进 innerHTML 成存储型 XSS 源(review)
    if not re.fullmatch(r"\d+", str(tmdb or "")):
        raise AppError("tmdbid 必须是数字: %r" % tmdb, status=400)
    # 记录旧绑定到 undo —— 绑定是唯一没 undo 的破坏性操作,误绑/批量绑错后能查回原 tmdb(review)
    try:
        from lib.undo import _undo_record
        cur = eget("/Items", {"Ids": item_id, "Fields": "ProviderIds,Name"}).get("Items", [{}])[0]
        old_tmdb = (cur.get("ProviderIds") or {}).get("Tmdb", "")
        if str(old_tmdb) != str(tmdb):
            _undo_record("rebind", {"id": item_id, "name": cur.get("Name") or name,
                                    "old_tmdb": old_tmdb, "new_tmdb": str(tmdb), "type": typ})
    except Exception:
        pass
    epost("/Items/RemoteSearch/Apply/%s" % item_id, body={"ProviderIds": {"Tmdb": str(tmdb)}})
    epost("/Items/%s/Refresh" % item_id, {"MetadataRefreshMode": "FullRefresh", "ImageRefreshMode": "FullRefresh",
                                          "ReplaceAllMetadata": "true", "ReplaceAllImages": "true"})
    for _ in range(6):
        time.sleep(4)
        it = eget("/Items", {"Ids": item_id, "Fields": "ProviderIds"}).get("Items", [{}])[0]
        if it.get("ImageTags", {}).get("Primary"):
            log("海报修复 %s -> tmdb %s ✓" % (it.get("Name"), tmdb))
            return {"ok": True, "name": it.get("Name"), "poster": True}
    for c in remote_search(item_id, name or "", typ):
        if c["tmdb"] == str(tmdb) and c["img"]:
            epost("/Items/%s/RemoteImages/Download" % item_id, {"Type": "Primary", "ImageUrl": c["img"]})
            time.sleep(4); break
    it = eget("/Items", {"Ids": item_id, "Fields": "ProviderIds"}).get("Items", [{}])[0]
    ok = bool(it.get("ImageTags", {}).get("Primary"))
    log("海报修复 %s -> tmdb %s %s" % (it.get("Name"), tmdb, "✓" if ok else "(图未拉到)"))
    return {"ok": True, "name": it.get("Name"), "poster": ok}


def refresh_series(series_id):
    """刷新该剧元数据(从 TMDb 拉最新集列表,缺的会变虚拟集)。"""
    from lib.logger import log
    code = epost("/Items/%s/Refresh" % series_id, {"MetadataRefreshMode": "Default", "ImageRefreshMode": "Default", "Recursive": "false", "ReplaceAllMetadata": "false"})
    log("刷新剧元数据 %s" % series_id)
    return {"ok": code in (200, 204), "code": code}


def list_noposter():
    out = []
    for name, m in fetch_libs().items():
        try:
            items = eget("/Items", {"ParentId": m["id"], "Recursive": "true", "IncludeItemTypes": "Series,Movie",
                                    "Fields": "ProviderIds,Path,ImageTags", "Limit": 30000}).get("Items", [])
        except Exception:
            continue
        for i in items:
            if not i.get("ImageTags", {}).get("Primary"):
                folder = (i.get("Path") or "").split("/" + m["folder"] + "/")[-1].split("/")[0]
                out.append({"id": i["Id"], "name": i["Name"], "lib": name, "type": i["Type"],
                            "tmdb": i.get("ProviderIds", {}).get("Tmdb", ""), "folder": folder})
    return out


def list_users(with_activity=False):
    """列出 Emby 用户;with_activity=True 多带 last_activity / last_login 字段。"""
    out = []
    try:
        for u in eget("/Users"):
            p = u.get("Policy", {})
            # 同时播放数:真实字段是 SimultaneousStreamLimit(MaxActiveSessions 在新版 Emby 是死字段,
            # 回落兼容)。码率上限 RemoteClientBitrateLimit 单位 bps,转 Mbps 给前端展示。
            stream_limit = p.get("SimultaneousStreamLimit") or p.get("MaxActiveSessions") or 0
            bps = p.get("RemoteClientBitrateLimit") or 0
            row = {"id": u["Id"], "name": u["Name"], "admin": bool(p.get("IsAdministrator")),
                   "disabled": bool(p.get("IsDisabled")),
                   "maxsessions": stream_limit,        # 兼容老前端字段名
                   "stream_limit": stream_limit,
                   "bitrate_mbps": round(bps / 1000000.0, 1) if bps else 0}
            if with_activity:
                row["last_activity"] = u.get("LastActivityDate", "") or ""
                row["last_login"] = u.get("LastLoginDate", "") or ""
            out.append(row)
    except Exception as e:
        return [{"id": "", "name": "读取失败: %s" % e, "admin": False, "disabled": False, "maxsessions": 0}]
    return out


def create_user(name, pw):
    from lib.logger import log
    name = (name or "").strip()
    if not name:
        return {"err": "用户名不能为空"}
    if any(u["Name"] == name for u in eget("/Users")):
        return {"err": "已存在同名用户"}
    epost("/Users/New", body={"Name": name})
    uid = next((u["Id"] for u in eget("/Users") if u["Name"] == name), None)
    if not uid:
        return {"err": "创建失败"}
    if pw:
        epost("/Users/%s/Password" % uid, body={"Id": uid, "CurrentPw": "", "NewPw": pw})
    log("新建 Emby 用户 %s" % name)
    return {"ok": True, "id": uid, "name": name}


def update_user(uid, maxsessions=None, disabled=None, bitrate_mbps=None):
    """改用户策略。
    maxsessions  → 同时播放数上限(写真实字段 SimultaneousStreamLimit,顺带 MaxActiveSessions 兼容旧版)
    bitrate_mbps → 远程串流码率上限(Mbps,转 bps 写 RemoteClientBitrateLimit;0/'' = 不限)
    disabled     → 停用/启用
    """
    from lib.logger import log
    pol = next((u.get("Policy", {}) for u in eget("/Users") if u["Id"] == uid), None)
    if pol is None:
        return {"err": "用户不存在"}
    if maxsessions is not None and str(maxsessions) != "":
        try:
            n = max(0, int(maxsessions))
            pol["SimultaneousStreamLimit"] = n
            pol["MaxActiveSessions"] = n  # 旧版 Emby 字段,新版忽略,写了无害
        except Exception: pass
    if bitrate_mbps is not None and str(bitrate_mbps) != "":
        try:
            mbps = max(0.0, float(bitrate_mbps))
            pol["RemoteClientBitrateLimit"] = int(round(mbps * 1000000))  # 0 = 不限
        except Exception: pass
    if disabled is not None:
        pol["IsDisabled"] = bool(disabled)
    code = epost("/Users/%s/Policy" % uid, body=pol)
    log("改用户策略 %s (同时播放=%s 码率Mbps=%s 停用=%s)" % (uid, maxsessions, bitrate_mbps, disabled))
    if code not in (200, 204):
        return {"ok": False, "code": code}
    # 写后回读校验:Emby 对不认识的 Policy 字段会静默忽略(MaxActiveSessions 那次事故),
    # 回读确认值真落库,避免"以为限了其实没限"。
    verify = {}
    try:
        np = next((u.get("Policy", {}) for u in eget("/Users") if u["Id"] == uid), {})
        # 只校验"设了限制(>0)"的;设 0=不限时 Emby 可能省略该字段,回读不必报警
        if maxsessions is not None and str(maxsessions) != "":
            want = max(0, int(maxsessions))
            if want:
                got = np.get("SimultaneousStreamLimit", np.get("MaxActiveSessions"))
                verify["stream_limit_ok"] = (got == want)
        if bitrate_mbps is not None and str(bitrate_mbps) != "":
            want_bps = int(round(max(0.0, float(bitrate_mbps)) * 1000000))
            if want_bps:
                verify["bitrate_ok"] = (np.get("RemoteClientBitrateLimit") == want_bps)
    except Exception:
        pass
    bad = [k for k, v in verify.items() if v is False]
    if bad:
        log("⚠️ 用户策略回读不符 %s: %s(Emby 可能不支持该字段)" % (uid, bad))
        return {"ok": True, "code": code, "verify": verify,
                "warn": "已提交但回读发现未生效: %s(该 Emby 版本可能不支持此限制)" % ", ".join(bad)}
    return {"ok": True, "code": code, "verify": verify}


def delete_user(uid):
    from lib.logger import log
    code = edelete("/Users/%s" % uid)
    log("删除 Emby 用户 %s" % uid)
    return {"ok": code in (200, 204), "code": code}
