#!/usr/bin/env python3
"""清理115上与已删除strm对应的残留文件/文件夹"""
import json, urllib.request, urllib.parse, os, subprocess, time

raw = subprocess.check_output(
    "echo Celeron123!@# | sudo -S cat /volume1/docker/emby-manager/config.json",
    shell=True
)
cfg = json.loads(raw)
COOKIE = cfg["c115_cookie"]
UA = "Mozilla/5.0"
CID = "3455363909050053299"

def req115(path, params=None):
    url = "https://webapi.115.com" + path
    if params:
        url += "?" + urllib.parse.urlencode(params)
    h = {"User-Agent": UA, "Cookie": COOKIE, "Referer": "https://115.com/", "Accept": "application/json"}
    r = urllib.request.Request(url, headers=h, method="GET")
    with urllib.request.urlopen(r, timeout=30) as resp:
        return json.loads(resp.read())

def req_post(path, body_dict):
    url = "https://webapi.115.com" + path
    data = urllib.parse.urlencode(body_dict).encode("utf-8")
    h = {"User-Agent": UA, "Cookie": COOKIE, "Referer": "https://115.com/",
         "Accept": "application/json", "Content-Type": "application/x-www-form-urlencoded"}
    r = urllib.request.Request(url, headers=h, data=data, method="POST")
    with urllib.request.urlopen(r, timeout=30) as resp:
        return json.loads(resp.read())

# 1. 115子文件夹
print(">>> 拉取115子文件夹...")
d115 = {}
offset = 0
while True:
    r = req115("/files", {"aid":"1","cid":CID,"o":"user_ptime","asc":"0",
                           "offset":offset,"limit":1000,"show_dir":1,"format":"json"})
    data = r.get("data") or []
    if not data: break
    for it in data:
        if not it.get("fid"):
            d115[str(it["cid"])] = it.get("n","")
    offset += len(data)

# 2. NAS strm文件夹
STRM = "/volume1/strm/最新电影"
d_nas = set()
if os.path.isdir(STRM):
    for d in os.listdir(STRM):
        if os.path.isdir(os.path.join(STRM, d)):
            d_nas.add(d)

# 3. 115有但NAS没有 = 待删除子文件夹
to_del_dirs = [(cid, name) for cid, name in d115.items() if name not in d_nas]
print("  应删子文件夹: {} 个".format(len(to_del_dirs)))
for cid, name in to_del_dirs:
    print("    D [{}] {}".format(cid, name[:70]))

# 4. 115根文件
print()
print(">>> 拉取115根文件...")
f115 = {}
offset = 0
while True:
    r = req115("/files", {"aid":"1","cid":CID,"o":"user_ptime","asc":"0",
                           "offset":offset,"limit":1000,"show_dir":0,"format":"json"})
    data = r.get("data") or []
    if not data: break
    for it in data:
        fid = it.get("fid")
        name = it.get("n","")
        if fid: f115[str(fid)] = name
    offset += len(data)

# 视频文件 → strm文件夹名
def video_stem(name):
    for ext in (".mkv",".mp4",".iso",".m2ts",".ts",".avi",".mov",".wmv",".flv",".rmvb"):
        if name.lower().endswith(ext):
            return name[:-len(ext)]
    return name

# 根文件：对应strm已删除 → 待删除
to_del_files = {}
d115_names = set(d115.values())
for fid, name in f115.items():
    stem = video_stem(name)
    if stem not in d_nas and stem not in d115_names:
        to_del_files[fid] = name

# 非视频文件也删
for fid, name in f115.items():
    ext = name.rsplit(".",1)[-1].lower() if "." in name else ""
    if ext in ("docx","txt"):
        if fid not in to_del_files:
            to_del_files[fid] = name

print("  应删根文件: {} 个".format(len(to_del_files)))
for fid, name in list(to_del_files.items())[:15]:
    print("    F [{}] {}".format(fid, name[:70]))

# ===== 执行删除 =====
print()
print("=" * 50)

# 先删子文件夹
ok_dirs = 0
for cid, name in to_del_dirs:
    try:
        r = req_post("/rb/delete", {"fid[0]": cid, "pid": CID, "ignore_warn": "1"})
        if r.get("state"):
            ok_dirs += 1
            print("  OK DIR {}".format(name[:60]))
        else:
            print("  FAIL DIR {}: {}".format(name[:40], r.get("error","")[:60]))
    except Exception as e:
        print("  ERR DIR {}: {}".format(name[:40], str(e)[:60]))
    time.sleep(0.5)

# 再删根文件
ok_files = 0
batch = []
for fid, name in to_del_files.items():
    batch.append(fid)
    if len(batch) >= 10:
        try:
            r = req_post("/rb/delete", {"fid[0]": batch[0], "pid": CID, "ignore_warn": "1"})
            if r.get("state"): ok_files += 1
        except: pass
        # 逐个删更可靠
        for bfid in batch:
            try:
                r = req_post("/rb/delete", {"fid[0]": bfid, "pid": CID, "ignore_warn": "1"})
                if r.get("state"): ok_files += 1
            except: pass
            time.sleep(0.3)
        batch = []
        time.sleep(0.3)

print()
print("删除完成: 文件夹{}个 / 文件{}个".format(ok_dirs, ok_files))
