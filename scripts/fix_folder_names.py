#!/usr/bin/env python3
"""从脏文件夹名提取真正电影名并重命名，让 Emby 能匹配 TMDb 刮海报。"""
import json, urllib.request, os, re, subprocess, time, sys

EMBY = "http://127.0.0.1:8096/emby"
KEY = "0faf87b4f47148c9b92cb9d580d4e734"
STRM = "/volume1/strm/最新电影"
LIB_ID = "150748"
SUDO_PW = "Celeron123!@#"

def eget(path, params=None):
    p = dict(params or {})
    p["api_key"] = KEY
    url = EMBY + path + "?" + urllib.parse.urlencode(p)
    with urllib.request.urlopen(url, timeout=60) as r:
        return json.loads(r.read())

def sudo_mv(src, dst):
    r = subprocess.run(["sudo", "-S", "mv", src, dst],
                       input=SUDO_PW.encode(), capture_output=True, timeout=10)
    return r.returncode == 0

def extract_movie_name(folder_name):
    """从 [电影名][标签][标签] 格式中提取干净的电影名+年份。

    返回 (clean_name, year) 或 (None, None)
    """
    # 策略1: 从 [] 标签中提取
    # 格式: [电影名][...其他标签...]
    # 或者: [电影名.英文名.年份][...]
    segments = folder_name.split("][")
    # 清理首尾括号
    if segments and segments[0].startswith("["):
        segments[0] = segments[0][1:]
    if segments and segments[-1].endswith("]"):
        segments[-1] = segments[-1][:-1]

    candidates = []
    year = None

    for seg in segments:
        seg = seg.strip()
        if not seg:
            continue

        # 找年份
        year_match = re.search(r'\b(19\d{2}|20\d{2})\b', seg)
        if year_match and not year:
            year = year_match.group(1)

        # 检查是否是纯中文名（电影名）
        if re.search(r'[一-鿿]{2,}', seg):
            # 去掉年份、分辨率等尾缀
            clean = re.sub(r'\s*\d{4}.*', '', seg)
            clean = re.sub(r'\s*[23][0-9]{3}[pP].*', '', clean)
            clean = re.sub(r'\s*4K.*', '', clean)
            clean = clean.strip()
            if len(clean) >= 2 and not all(c in '0123456789pPiIkK' for c in clean):
                candidates.append(clean)
                break  # 第一个中文段通常是电影名

    # 策略2: 如果有英文名，用英文名
    if not candidates:
        for seg in segments:
            seg = seg.strip()
            # 找 "Movie.Name.2025" 模式
            m = re.match(r'^([A-Za-z][A-Za-z0-9.&!]+?)\s*(\d{4})?', seg)
            if m:
                name = m.group(1).replace(".", " ").strip()
                if name and len(name) > 3:
                    candidates.append(name)
                    if m.group(2) and not year:
                        year = m.group(2)
                    break

    if not candidates:
        return None, None

    name = candidates[0]
    return name, year

# ====== 主流程 ======
print(">>> 拉取无海报无TMDb的电影...")
no_match = []
offset = 0
while True:
    items = eget("/Items", {
        "ParentId": LIB_ID, "Recursive": "true",
        "IncludeItemTypes": "Movie",
        "Fields": "ImageTags,ProviderIds,Name,Path,ProductionYear",
        "Limit": 200, "StartIndex": offset,
    })
    chunk = items.get("Items", [])
    if not chunk:
        break
    for i in chunk:
        has_img = bool(i.get("ImageTags", {}).get("Primary"))
        tmdb = (i.get("ProviderIds") or {}).get("Tmdb", "")
        if not has_img and not tmdb:
            path = i.get("Path", "")
            folder = path.replace("/strm/最新电影/", "").split("/")[0] if "/strm/" in path else ""
            no_match.append({
                "id": i.get("Id"),
                "name": i.get("Name", ""),
                "year": i.get("ProductionYear"),
                "path": path,
                "folder": folder,
            })
    offset += len(chunk)
    if len(chunk) < 200:
        break

print("  共 {} 部\n".format(len(no_match)))

# 分析每个文件夹名，看能否提取
can_extract = []
cannot = []
for m in no_match:
    clean, year = extract_movie_name(m["folder"])
    if clean:
        can_extract.append({**m, "clean": clean, "extracted_year": year})
    else:
        cannot.append(m)

print("可提取电影名: {} 个".format(len(can_extract)))
print("无法提取: {} 个".format(len(cannot)))

if "--dry-run" in sys.argv:
    print()
    print("=== 预览重命名 (可提取的) ===")
    for m in can_extract[:30]:
        new_name = m["clean"]
        if m["extracted_year"]:
            new_name += " ({})".format(m["extracted_year"])
        print("  {} → {}".format(m["folder"][:60], new_name[:60]))
    print()
    print("=== 无法提取的 ===")
    for m in cannot[:15]:
        print("  {}".format(m["folder"][:80]))
    sys.exit(0)

# ====== 执行重命名 ======
print()
print(">>> 执行重命名...")
renamed = 0
failed = 0

for m in can_extract:
    old_folder = os.path.join(STRM, m["folder"])
    new_name = m["clean"]
    if m["extracted_year"]:
        new_name += " ({})".format(m["extracted_year"])
    new_folder = os.path.join(STRM, new_name)

    if old_folder == new_folder:
        continue

    if os.path.isdir(new_folder):
        print("  SKIP (目标已存在): {} → {}".format(m["folder"][:50], new_name[:50]))
        failed += 1
        continue

    if sudo_mv(old_folder, new_folder):
        renamed += 1
        if renamed % 20 == 0:
            print("  [{}/{}]".format(renamed, len(can_extract)))
    else:
        failed += 1
        print("  FAIL: {} → {}".format(m["folder"][:50], new_name[:50]))
    time.sleep(0.1)

print()
print("重命名完成: {} ok / {} fail".format(renamed, failed))

# ====== 触发 Emby 扫描 ======
if renamed > 0:
    print()
    print(">>> 触发 Emby 库扫描...")
    url = "{}Library/Refresh?id={}&Recursive=true&api_key={}".format(EMBY, LIB_ID, KEY)
    req = urllib.request.Request(url, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            print("  扫描已触发: {}".format(r.getcode()))
    except Exception as e:
        print("  触发失败: {}".format(e))
