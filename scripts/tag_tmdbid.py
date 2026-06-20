#!/usr/bin/env python3
"""给 最新电影 中没有 [tmdbid] 标记的 strm 文件夹加上标记，让去重算法能识别。"""
import sys, os, re, json, time, shutil
sys.path.insert(0, '/volume1/docker/emby-manager')
from lib.emby import eget

STRM = "/volume1/strm/最新电影"
EMBY = "http://127.0.0.1:8096/emby"
KEY = "0faf87b4f47148c9b92cb9d580d4e734"
LIB_ID = "150748"

# 1. 收集无标记的文件夹
to_tag = {}
for d in os.listdir(STRM):
    dp = os.path.join(STRM, d)
    if not os.path.isdir(dp):
        continue
    if re.search(r'tmdbid[-_]\d+', d, re.I):
        continue  # 已有标记
    to_tag[d] = None  # 待填充 tmdbid

print("无标记文件夹: {} 个".format(len(to_tag)))

# 2. 从 Emby 获取每个文件夹对应的 TMDb ID
print("从 Emby 查 TMDb...")
offset = 0
found = 0
while True:
    items = eget("/Items", {
        "ParentId": LIB_ID, "Recursive": "true",
        "IncludeItemTypes": "Movie",
        "Fields": "Path,ProviderIds",
        "Limit": 200, "StartIndex": offset,
    })
    chunk = items.get("Items", [])
    if not chunk:
        break
    for i in chunk:
        path = i.get("Path", "")
        folder = path.replace("/strm/最新电影/", "").split("/")[0]
        tmdb = (i.get("ProviderIds") or {}).get("Tmdb", "")
        if folder in to_tag and tmdb:
            to_tag[folder] = tmdb
            found += 1
    offset += len(chunk)

print("  找到 TMDb: {} / {}".format(found, len(to_tag)))

# 统计无 TMDb 的
no_tmdb = [d for d, t in to_tag.items() if not t]
if no_tmdb:
    print("  无 TMDb: {} 个 (跳过)".format(len(no_tmdb)))

# 3. 重命名
print()
print("重命名...")
ok = 0
for old_name, tmdb in to_tag.items():
    if not tmdb:
        continue
    # 新名称: 原名 + [tmdbid-XXX]
    new_name = old_name + "[tmdbid-" + str(tmdb) + "]"
    if new_name == old_name:
        continue
    old_path = os.path.join(STRM, old_name)
    new_path = os.path.join(STRM, new_name)
    if os.path.exists(new_path):
        continue  # 目标已存在
    try:
        shutil.move(old_path, new_path)
        ok += 1
        if ok % 50 == 0:
            print("  [{}/{}]".format(ok, len(to_tag) - len(no_tmdb)))
    except Exception as e:
        print("  FAIL: {} → {} : {}".format(old_name[:40], new_name[:40], e))

print("重命名: {} ok".format(ok))
print()
print("旧名例: {}".format(list(to_tag.keys())[0][:60]))
print("新名例: {}[tmdbid-{}]".format(list(to_tag.keys())[0][:60], list(to_tag.values())[0]))

# 4. 触发扫描
import urllib.request
url = "{}Library/Refresh?id={}&Recursive=true&api_key={}".format(EMBY, LIB_ID, KEY)
req = urllib.request.Request(url, method="POST")
with urllib.request.urlopen(req, timeout=30) as r:
    print("扫描: {}".format(r.getcode()))
