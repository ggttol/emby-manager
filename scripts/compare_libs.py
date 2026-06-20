#!/usr/bin/env python3
"""对比 115 网盘目录 vs Emby 库，找出未入库项和死链接。"""
import json, urllib.request, urllib.parse, subprocess, re, sys

# ==== 读取 config ====
raw = subprocess.check_output(
    "echo Celeron123!@# | sudo -S cat /volume1/docker/emby-manager/config.json",
    shell=True
)
cfg = json.loads(raw)
COOKIE = cfg["c115_cookie"]
EMBY = cfg["emby_url"]
KEY = cfg["api_key"]
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36"

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

# ==== 1. 从 115 拉所有子文件夹名 ====
print("拉取 115 最新电影子文件夹...")
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

# ==== 2. 从 Emby 拉所有 Movie 及其 Path ====
print("拉取 Emby 最新电影库条目...")
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
        "Fields": "Path,ProductionYear",
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
            "path": path,
            "folder": folder,
            "id": i.get("Id"),
        })
    offset += len(chunk)
    if len(chunk) < 200:
        break
print("  Emby 条目: {} 个".format(len(emby_items)))

# ==== 3. 交叉对比 ====
# 115 文件夹名 → cid
f115_map = {}
for f in f115_folders:
    f115_map[f["name"]] = f["cid"]

# Emby folder → item
emby_by_folder = {}
for it in emby_items:
    emby_by_folder[it["folder"]] = it

# 匹配逻辑：
# 精确匹配：115文件夹名 == Emby folder名
# Emby folder 出现在 115 文件夹名中
# 115 文件夹名关键词出现在 Emby folder 中

def clean_for_match(s):
    """去除常见标签，提取核心名"""
    s = re.sub(r'\[.*?\]', '', s)
    s = re.sub(r'【.*?】', '', s)
    s = re.sub(r'（.*?）', '', s)
    s = re.sub(r'\(.*?\)', '', s)
    s = re.sub(r'\b(4K|1080p|2160p|BluRay|Blu-Ray|WEB-DL|WEB|HDR|DV|DTS|Atmos|H\.?265|HEVC|AAC|DDP?\d\.\d|60[fF][pP][sS]|HDR10\+?|SDR|Remux|DIY|原盘|高码|纯净版|内封|简繁|字幕|中字|HDSky|HDHome|CHDBits|TTG|HDSWEB|HHWEB|ADWeb|DDP\d\.\d|LPCM|BDMV|H\.264|PandaQT|ColorWEB)\b', '', s, flags=re.I)
    s = re.sub(r'\d{4}[\.\-]\d{2}[\.\-]\d{2}', '', s)
    s = re.sub(r'\d{2,3}\.\d{2,3}GB', '', s)
    s = re.sub(r'\d+\.\d+Mbps', '', s)
    s = re.sub(r'[【】《》「」\s]+', ' ', s)
    s = re.sub(r'\s+', ' ', s).strip()
    return s

matched_115 = set()
matched_emby = set()

# Pass 1: 精确匹配
for ef, eit in emby_by_folder.items():
    if ef in f115_map:
        matched_115.add(ef)
        matched_emby.add(ef)

# Pass 2: 115文件夹名包含Emby folder名
for ef, eit in emby_by_folder.items():
    if ef in matched_emby:
        continue
    for fn in f115_map:
        if fn in matched_115:
            continue
        if ef and len(ef) >= 3 and ef in fn:
            matched_115.add(fn)
            matched_emby.add(ef)
            break

# Pass 3: Emby folder 包含 115 文件夹名的核心关键词
for ef, eit in emby_by_folder.items():
    if ef in matched_emby:
        continue
    ef_clean = clean_for_match(ef)
    for fn in f115_map:
        if fn in matched_115:
            continue
        fn_clean = clean_for_match(fn)
        if fn_clean and len(fn_clean) >= 3 and fn_clean in ef_clean:
            matched_115.add(fn)
            matched_emby.add(ef)
            break

# Pass 4: 双向包含（115核心词 在 emby folder 中，或反过来）
for ef, eit in emby_by_folder.items():
    if ef in matched_emby:
        continue
    ef_clean = clean_for_match(ef)
    for fn in f115_map:
        if fn in matched_115:
            continue
        fn_clean = clean_for_match(fn)
        if ef_clean and fn_clean and len(fn_clean) >= 3 and len(ef_clean) >= 3:
            if fn_clean in ef_clean or ef_clean in fn_clean:
                matched_115.add(fn)
                matched_emby.add(ef)
                break

# 结果
f115_unmatched = [(f["name"], f["cid"]) for f in f115_folders if f["name"] not in matched_115]
emby_unmatched = [it for it in emby_items if it["folder"] not in matched_emby]

# ==== 4. 输出报告 ====
print()
print("=" * 70)
print("  对比报告：115 网盘 vs Emby「最新电影」库")
print("=" * 70)
print()
print("| 指标 | 数量 |")
print("|------|------|")
print("| 115 子文件夹 | {} |".format(len(f115_folders)))
print("| Emby Movie 条目 | {} |".format(len(emby_items)))
print("| 匹配成功 | {} |".format(len(matched_115)))
print("| **115 有 / Emby 无 (未入库)** | **{}** |".format(len(f115_unmatched)))
print("| **Emby 有 / 115 无 (死链接)** | **{}** |".format(len(emby_unmatched)))

if f115_unmatched:
    print()
    print("=" * 70)
    print("  🔴 115 有但 Emby 没有 — 未入库的文件夹 ({} 个)".format(len(f115_unmatched)))
    print("=" * 70)
    for name, cid in f115_unmatched:
        print("  📁 {}  (cid={})".format(name[:90], cid))

if emby_unmatched:
    print()
    print("=" * 70)
    print("  🔴 Emby 有但 115 没有 — 死链接/源文件已删 ({} 个)".format(len(emby_unmatched)))
    print("=" * 70)
    for it in emby_unmatched:
        print("  🎬 {} ({}) — folder: {}".format(it["name"], it["year"] or "?", it["folder"][:80]))
