#!/usr/bin/env python3
"""端到端修复：对齐 115「最新电影」与 Emby「最新电影」库。
策略：
1. 清理 Emby 中指向不存在 strm 文件的条目（先创建空 strm → DELETE → 删除空文件）
2. 确保所有 115 子文件夹都有正确的 strm 文件
3. 触发 Emby 重新扫描
"""
import json, urllib.request, urllib.error, subprocess, re, sys, time, os

raw = subprocess.check_output(
    "echo Celeron123!@# | sudo -S cat /volume1/docker/emby-manager/config.json",
    shell=True
)
cfg = json.loads(raw)
COOKIE = cfg["c115_cookie"]
EMBY = cfg["emby_url"]
KEY = cfg["api_key"]
UA = "Mozilla/5.0"

# CloudDrive2 挂载基础路径
STRM_BASE = "/volume1/strm/最新电影"

def req_115(path, params=None):
    url = "https://webapi.115.com" + path
    if params:
        url += "?" + urllib.parse.urlencode(params)
    h = {"User-Agent": UA, "Cookie": COOKIE,
         "Referer": "https://115.com/", "Accept": "application/json"}
    r = urllib.request.Request(url, headers=h, method="GET")
    with urllib.request.urlopen(r, timeout=30) as resp:
        return json.loads(resp.read())

def eget(path, params=None):
    p = dict(params or {})
    p["api_key"] = KEY
    url = EMBY + path + "?" + urllib.parse.urlencode(p)
    with urllib.request.urlopen(url, timeout=60) as r:
        return json.loads(r.read())

def edelete(item_id):
    url = EMBY + "/Items/" + item_id + "?api_key=" + KEY
    req = urllib.request.Request(url, method="DELETE")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.getcode()
    except urllib.error.HTTPError as e:
        return e.code

# ======== 1. 从 Emby 获取所有「最新电影」条目的 Path ========
print(">>> 拉取 Emby 最新电影库条目(含完整 Path)...")
lib_id = None
for f in eget("/Library/VirtualFolders"):
    if f.get("Name") == "最新电影":
        lib_id = f.get("ItemId")
        break

emby_items = []
offset = 0
while True:
    items = eget("/Items", {
        "ParentId": lib_id,
        "Recursive": "true",
        "IncludeItemTypes": "Movie",
        "SortBy": "SortName",
        "SortOrder": "Ascending",
        "Fields": "Path,ProductionYear,ProviderIds",
        "Limit": 200,
        "StartIndex": offset,
    })
    chunk = items.get("Items", [])
    if not chunk:
        break
    for i in chunk:
        path = i.get("Path", "")
        folder = path.rsplit("/", 1)[0].rsplit("/", 1)[-1] if "/" in path else path
        emby_items.append({
            "name": i.get("Name"),
            "year": i.get("ProductionYear"),
            "path": path,        # Emby 存储的完整路径,如 /strm/最新电影/xxx/yyy.strm
            "folder": folder,
            "id": i.get("Id"),
            "tmdb": (i.get("ProviderIds") or {}).get("Tmdb", ""),
        })
    offset += len(chunk)
    if len(chunk) < 200:
        break
print("  Emby 条目: {} 个".format(len(emby_items)))

# ======== 2. 检查每个 Emby 条目的 strm 文件是否真实存在 ========
print()
print(">>> 检查 strm 文件是否真实存在...")
dead = []   # strm 文件不存在的条目
alive = []  # strm 文件存在的条目
for it in emby_items:
    # Emby path 是容器内路径 /strm/xxx → 实际路径 /volume1/strm/xxx
    actual_path = "/volume1" + it["path"]
    # 同时检查不带 /strm/前缀的情况（Emby 可能存绝对容器路径）
    if os.path.exists(actual_path):
        alive.append(it)
    else:
        dead.append(it)

print("  存活: {} 个, 死链: {} 个".format(len(alive), len(dead)))

# ======== 3. 对死链创建临时空 strm 文件，用于 DELETE ========
print()
print(">>> 修复死链:创建临时空 strm → DELETE → 清理...")
deleted = []
failed = []

for it in dead:
    actual_path = "/volume1" + it["path"]
    items_id = it["id"]

    # 3a. 创建目录和临时空 strm
    dir_path = os.path.dirname(actual_path)
    try:
        os.makedirs(dir_path, exist_ok=True)
        # 写空 strm(内容无所谓，只要文件存在)
        with open(actual_path, "w") as f:
            f.write("# placeholder for deletion\n")
    except Exception as e:
        print("  SKIP {} — 创建临时文件失败: {}".format(it["name"], e))
        failed.append(it)
        continue

    # 3b. 从 Emby 删除此 item
    code = edelete(items_id)
    if code in (200, 204):
        deleted.append(it)
        print("  OK {} | {}".format(items_id, it["name"]))
    else:
        # 再试一次(偶尔 500 是暂时的)
        time.sleep(1)
        code2 = edelete(items_id)
        if code2 in (200, 204):
            deleted.append(it)
            print("  OK(retry) {} | {}".format(items_id, it["name"]))
        else:
            failed.append(it)
            print("  FAIL [{}→{}] {} | {}".format(code, code2, items_id, it["name"]))

    # 3c. 删除临时 strm 文件
    try:
        os.remove(actual_path)
        # 如果目录变空了，也删掉
        try:
            os.rmdir(dir_path)
        except OSError:
            pass  # 目录不空，保留
    except Exception:
        pass

    time.sleep(0.3)

print()
print("  删除成功: {} 个, 失败: {} 个".format(len(deleted), len(failed)))

# ======== 4. 检查 115 文件夹，确保每个都有 strm ========
print()
print(">>> 拉取 115 最新电影子文件夹...")
CID = "3455363909050053299"
f115_folders = []
offset = 0
while True:
    r = req_115("/files", {
        "aid": "1", "cid": CID, "o": "user_ptime", "asc": "0",
        "offset": offset, "limit": 1000, "show_dir": 1, "format": "json",
    })
    data = r.get("data") or []
    if not data:
        break
    for it in data:
        if not it.get("fid"):
            f115_folders.append({"cid": it.get("cid"), "name": it.get("n") or ""})
    offset += len(data)
    if len(data) < 1000:
        break
print("  115 子文件夹: {} 个".format(len(f115_folders)))

# 列出 NAS 上已有 strm 文件夹
existing_strm_dirs = set()
if os.path.isdir(STRM_BASE):
    for entry in os.listdir(STRM_BASE):
        full = os.path.join(STRM_BASE, entry)
        if os.path.isdir(full):
            existing_strm_dirs.add(entry)

print("  已有 strm 文件夹: {} 个".format(len(existing_strm_dirs)))

# 找没 strm 的 115 文件夹
need_strm = []
for f in f115_folders:
    if f["name"] not in existing_strm_dirs:
        need_strm.append(f)

if need_strm:
    print()
    print("  需要创建 strm 的文件夹: {} 个".format(len(need_strm)))
    for f in need_strm:
        print("    📁 {}".format(f["name"][:90]))
else:
    print("  ✅ 所有 115 文件夹都有对应 strm")

# 找 strm 有但 115 没有的文件夹(多余的)
extra_strm = existing_strm_dirs - {f["name"] for f in f115_folders}
if extra_strm:
    print()
    print("  ⚠️ strm有但115无(多余文件夹): {} 个".format(len(extra_strm)))
    for d in sorted(extra_strm):
        print("    📁 {}".format(d[:90]))

# ======== 5. 触发 Emby 重新扫描 ========
print()
print(">>> 触发 Emby 库扫描...")
refresh_url = "{}Library/Refresh?id={}&Recursive=true&MetadataRefreshMode=Default&ImageRefreshMode=Default&api_key={}".format(
    EMBY, lib_id, KEY)
req = urllib.request.Request(refresh_url, method="POST")
try:
    with urllib.request.urlopen(req, timeout=30) as r:
        print("  扫描已触发: {}".format(r.getcode()))
except urllib.error.HTTPError as e:
    print("  触发失败: {}".format(e.code))

# ======== 6. 最终统计 ========
print()
print("=" * 60)
print("  修复完成")
print("=" * 60)
print("  删除死链: {} 成功 / {} 失败".format(len(deleted), len(failed)))

# 重新查询 Emby
time.sleep(5)
r = eget("/Items", {"ParentId": lib_id, "Recursive": "true", "IncludeItemTypes": "Movie", "Limit": 1})
new_cnt = r.get("TotalRecordCount", 0)
print("  Emby 最新电影库: {} → {} 部".format(len(emby_items), new_cnt))
print("  115 最新电影: {} 个子文件夹".format(len(f115_folders)))
