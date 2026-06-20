#!/usr/bin/env python3
"""将 115「最新电影」根目录的散落视频文件批量生成 strm 入库 Emby。"""
import json, urllib.request, urllib.parse, subprocess, os, re, time, sys

raw = subprocess.check_output(
    "echo Celeron123!@# | sudo -S cat /volume1/docker/emby-manager/config.json",
    shell=True
)
cfg = json.loads(raw)
COOKIE = cfg["c115_cookie"]
UA = "Mozilla/5.0"
CID = "3455363909050053299"
STRM_BASE = "/volume1/strm/最新电影"
SUDO_PW = "Celeron123!@#"

# 视频扩展名
VIDEO_EXT = {".mkv", ".mp4", ".iso", ".m2ts", ".ts", ".avi", ".mov", ".wmv", ".flv", ".rmvb"}
# 要跳过的非视频扩展名
SKIP_EXT = {".docx", ".txt", ".jpg", ".jpeg", ".png", ".webp", ".srt", ".ass", ".sup",
            ".clpi", ".mpls", ".bdmv", ".bdjo", ".nfo", ".xml", ".jar", ".bin",
            ".crt", ".crl", ".sig", ".sfv", ".md5"}

def req_115(path, params=None):
    url = "https://webapi.115.com" + path
    if params:
        url += "?" + urllib.parse.urlencode(params)
    h = {"User-Agent": UA, "Cookie": COOKIE,
         "Referer": "https://115.com/", "Accept": "application/json"}
    r = urllib.request.Request(url, headers=h, method="GET")
    with urllib.request.urlopen(r, timeout=30) as resp:
        return json.loads(resp.read())

def write_strm(path, content):
    """写 strm 文件（进程以 root 运行，直接写）"""
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)
        return True
    except Exception as e:
        print("  write_strm err: {}".format(e))
        return False

def clean_name(filename):
    """从视频文件名提取干净的文件夹名"""
    # 去扩展名
    name = filename
    for ext in VIDEO_EXT:
        if name.lower().endswith(ext):
            name = name[:-len(ext)]
            break
    # 去除常见后缀标记
    name = re.sub(r'\.[A-Za-z0-9]+$', '', name)  # 去掉最后的 .xxx 后缀
    # 限制长度
    if len(name) > 180:
        name = name[:180]
    return name.strip()

# ====== 1. 扫描 115 根目录视频文件 ======
print(">>> 扫描 115 最新电影根目录...")
all_files = []
offset = 0
while True:
    r = req_115("/files", {
        "aid": "1", "cid": CID, "o": "user_ptime", "asc": "0",
        "offset": offset, "limit": 1000, "show_dir": 0, "format": "json",
    })
    data = r.get("data") or []
    if not data:
        break
    for it in data:
        name = it.get("n") or ""
        sz = int(it.get("s") or 0)
        fid = it.get("fid")
        all_files.append({"name": name, "size": sz, "fid": fid})
    offset += len(data)
    if len(data) < 1000:
        break

# 过滤：只要视频文件
# BDMV 碎片: 纯数字命名的 .m2ts 是蓝光原盘流片段,跳过
BDMV_FRAGMENT = re.compile(r'^\d{4,5}\.m2ts$', re.I)

videos = []
skipped = {}
skipped_bdmv = 0
for f in all_files:
    name = f["name"]
    name_lower = name.lower()
    ext = "." + name_lower.rsplit(".", 1)[-1] if "." in name_lower else ""
    if ext in VIDEO_EXT:
        # 跳过 BDMV 数字碎片
        if ext == ".m2ts" and BDMV_FRAGMENT.match(name):
            skipped_bdmv += 1
            continue
        # 跳过 .ts 的 BDMV 碎片
        if ext == ".ts" and re.match(r'^\d{4,5}\.ts$', name, re.I):
            skipped_bdmv += 1
            continue
        videos.append(f)
    else:
        ext_key = ext or "(无扩展名)"
        skipped[ext_key] = skipped.get(ext_key, 0) + 1

print("  总文件: {} 个".format(len(all_files)))
print("  视频文件: {} 个".format(len(videos)))
print("  跳过 BDMV碎片: {} 个".format(skipped_bdmv))
print("  跳过非视频: {} 个".format(sum(skipped.values())))
for ext, cnt in sorted(skipped.items(), key=lambda x: -x[1]):
    print("    {} : {} 个".format(ext, cnt))

# ====== 2. 查找已有的 strm 文件夹（避免重复）=====
existing = set()
if os.path.isdir(STRM_BASE):
    for d in os.listdir(STRM_BASE):
        if os.path.isdir(os.path.join(STRM_BASE, d)):
            existing.add(d)

print()
print("  已有 strm 文件夹: {} 个".format(len(existing)))

# ====== 3. 去重：同名文件夹跳过 ======
to_import = []
dup_count = 0
for v in videos:
    folder = clean_name(v["name"])
    if folder in existing:
        dup_count += 1
    else:
        to_import.append(v)
        existing.add(folder)  # 防止同批次内的同名

print("  重复跳过: {} 个".format(dup_count))
print("  待导入: {} 个".format(len(to_import)))

if not to_import:
    print("\n没有需要导入的文件")
    sys.exit(0)

# ====== 4. 确认执行 ======
total_gb = sum(v["size"] for v in to_import) / 1073741824
print()
print("=" * 60)
print("  将导入 {} 个视频文件（合计 {:.1f} GB）".format(len(to_import), total_gb))
print("  预计耗时: ~{:.0f} 分钟".format(len(to_import) * 0.8 / 60))
print("=" * 60)

if "--dry-run" in sys.argv:
    print("\n[DRY RUN] 以下是将创建的 strm (前20个):")
    for v in to_import[:20]:
        folder = clean_name(v["name"])
        print("  {} -> {}".format(folder[:60], v["name"][:60]))
    sys.exit(0)

# ====== 5. 逐个创建 strm ======
ok, fail = 0, 0
t0 = time.time()

for idx, v in enumerate(to_import):
    folder = clean_name(v["name"])
    video_name = v["name"]

    # strm 文件夹路径
    strm_dir = os.path.join(STRM_BASE, folder)
    # strm 文件路径（与视频同名但加 .strm）
    strm_file = os.path.join(strm_dir, video_name + ".strm")
    # strm 内容：Emby 容器内路径
    strm_content = "/media/最新电影/" + video_name

    try:
        # 创建文件夹 + 写 strm 文件
        if write_strm(strm_file, strm_content):
            ok += 1
            if (ok % 50) == 0:
                elapsed = time.time() - t0
                pct = ok * 100 / len(to_import)
                eta = elapsed / ok * (len(to_import) - ok)
                print("  [{}/{}] {:.0f}%  ETA: {:.0f}s".format(ok, len(to_import), pct, eta))
        else:
            fail += 1
            print("  FAIL write: {}".format(folder[:60]))

    except Exception as e:
        fail += 1
        print("  ERR {} : {}".format(folder[:60], e))

    # 防风控：每文件间隔 0.5s
    time.sleep(0.5)

elapsed = time.time() - t0
print()
print("=" * 60)
print("  导入完成: {} ok / {} fail".format(ok, fail))
print("  耗时: {:.0f} 秒".format(elapsed))
print("=" * 60)

# ====== 6. 触发 Emby 扫描 ======
if ok > 0:
    print()
    print(">>> 触发 Emby 库扫描...")
    EMBY = cfg["emby_url"]
    KEY = cfg["api_key"]
    url = "{}Library/Refresh?id=150748&Recursive=true&api_key={}".format(EMBY, KEY)
    req = urllib.request.Request(url, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            print("  扫描已触发: {}".format(r.getcode()))
    except urllib.error.HTTPError as e:
        print("  触发失败: {}".format(e.code))
