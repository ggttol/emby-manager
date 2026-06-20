#!/usr/bin/env python3
"""Update catalog_115.db from a Tencent Docs spreadsheet already loaded in Chrome.

This project has no third-party dependencies, so the updater reads the online
docs.qq.com response blocks that Chrome cached after a logged-in browser opens
the spreadsheet. It does not read Chrome cookies or localStorage.

Typical flow:
  1. Open https://docs.qq.com/sheet/DZWtEeFFGZW9XUkJo?tab=000001 in logged-in Chrome.
  2. Make every sheet load once (Codex can automate normal tab navigation).
  3. Run:
       python3 scripts/update_catalog_from_qqdocs.py --dry-run
       python3 scripts/update_catalog_from_qqdocs.py
"""
import argparse
import base64
import collections
import html
import json
import os
import re
import shutil
import sqlite3
import sys
import time
import zlib
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_DB = ROOT / "catalog_115.db"
DEFAULT_DOC_ID = "DZWtEeFFGZW9XUkJo"
DEFAULT_PAD_MARK = "ekDxQFeoWRBh"
DEFAULT_MIN_ROWS = 100000
DEFAULT_SINCE_HOURS = 24

URL_RE = re.compile(rb"https://docs\.qq\.com/dop-api/(?:opendoc|get/sheet)[^\x00\r\n ]+")
LINK_RE = re.compile(r"(?:https?://|magnet:\?|ed2k://)[^\s<>\"'\u3000]+", re.I)
PKG_RE = re.compile(r"(合集|整包|全套|全集|打包)")
BAD_NAME_RE = re.compile(
    r"(快捷键|搜索资源|资源列表|点击返回|返回|链接|地址|提取码|访问码|password|115cdn\.com|magnet:\?|ed2k://)",
    re.I,
)
SIZE_RE = re.compile(r"^\d+(?:\.\d+)?\s*(?:GB|MB|TB|G|M|T)$", re.I)
DATE_RE = re.compile(r"^\d{4}[-/.年]\d{1,2}(?:[-/.月]\d{1,2})?")
TYPE_WORDS = re.compile(
    r"^(类型|格式|大小|日期|时间|年份|来源|字幕|分辨率|集数|备注|链接|下载|资源链接|磁力|ed2k|115)$",
    re.I,
)
STYLE_SKIP = {
    "宋体",
    "雅黑",
    "微软雅黑",
    "黑体",
    "Arial",
    "Calibri",
    "等线",
    "楷体",
    "仿宋",
    "Verdana",
    "Tahoma",
    "normal",
    "bold",
    "italic",
    "left",
    "right",
    "center",
    "General",
}
HEADER_HINTS = (
    "影视名称",
    "资源名称",
    "剧名",
    "片名",
    "影片名称",
    "电影名称",
    "名称",
    "标题",
    "影视名",
    "电影名",
    "资源名",
    "影片名",
)


class ProtoMsg:
    __slots__ = ("fields",)

    def __init__(self):
        self.fields = collections.defaultdict(list)


def read_varint(buf, idx):
    out = 0
    shift = 0
    while idx < len(buf):
        byte = buf[idx]
        idx += 1
        out |= (byte & 0x7F) << shift
        if not (byte & 0x80):
            return out, idx
        shift += 7
        if shift > 70:
            raise ValueError("varint too long")
    raise ValueError("varint eof")


def parse_msg(buf):
    msg = ProtoMsg()
    idx = 0
    while idx < len(buf):
        tag, idx = read_varint(buf, idx)
        field_no = tag >> 3
        wire_type = tag & 7
        if field_no <= 0:
            raise ValueError("bad field")
        if wire_type == 0:
            value, idx = read_varint(buf, idx)
            msg.fields[field_no].append(("v", value))
        elif wire_type == 1:
            if idx + 8 > len(buf):
                raise ValueError("eof64")
            msg.fields[field_no].append(("b", buf[idx : idx + 8]))
            idx += 8
        elif wire_type == 2:
            length, idx = read_varint(buf, idx)
            if idx + length > len(buf):
                raise ValueError("eoflen")
            msg.fields[field_no].append(("b", buf[idx : idx + length]))
            idx += length
        elif wire_type == 5:
            if idx + 4 > len(buf):
                raise ValueError("eof32")
            msg.fields[field_no].append(("b", buf[idx : idx + 4]))
            idx += 4
        else:
            raise ValueError("unsupported wire type %s" % wire_type)
    return msg


def values(msg, field_no):
    return [v for kind, v in msg.fields.get(field_no, []) if kind == "v"]


def byte_values(msg, field_no):
    return [v for kind, v in msg.fields.get(field_no, []) if kind == "b"]


def child_msgs(msg, field_no):
    out = []
    for blob in byte_values(msg, field_no):
        try:
            out.append(parse_msg(blob))
        except Exception:
            pass
    return out


def cell_ref_index(cell_data):
    direct = values(cell_data, 2)
    if direct:
        return direct[0]
    nested = byte_values(cell_data, 2)
    if not nested or not nested[0]:
        return 0
    try:
        msg = parse_msg(nested[0])
        refs = values(msg, 1)
        return refs[0] if refs else 0
    except Exception:
        return 0


def decode_utf8(blob):
    try:
        text = blob.decode("utf-8")
    except Exception:
        return ""
    if not text:
        return ""
    printable = sum(1 for ch in text if ch.isprintable() or ch in "\r\n\t")
    if printable / max(1, len(text)) < 0.65:
        return ""
    return text


def clean_text(text):
    if not text:
        return ""
    text = html.unescape(str(text)).replace("&amp;", "&").replace("&amp", "&")
    text = "".join(" " if (ord(ch) < 32 or ord(ch) == 127) else ch for ch in text)
    return re.sub(r"\s+", " ", text).strip()


def collect_strings(msg, depth=0):
    if depth > 8:
        return []
    out = []
    for items in msg.fields.values():
        for kind, value in items:
            if kind != "b":
                continue
            text = decode_utf8(value)
            if text:
                out.append(text)
            try:
                out.extend(collect_strings(parse_msg(value), depth + 1))
            except Exception:
                pass
    return out


def meaningful_chunks(chunks):
    out = []
    seen = set()
    for chunk in chunks:
        chunk = clean_text(chunk)
        if not chunk or chunk in seen:
            continue
        if chunk in STYLE_SKIP:
            continue
        if re.fullmatch(r"[A-Fa-f0-9]{6,8}", chunk):
            continue
        if re.fullmatch(r"[\d.\-]+", chunk) and len(chunk) < 8:
            continue
        if len(chunk) <= 1:
            continue
        out.append(chunk)
        seen.add(chunk)
    return out


def rich_text(blob):
    chunks = []
    direct = decode_utf8(blob)
    if direct:
        chunks.append(direct)
    try:
        chunks.extend(collect_strings(parse_msg(blob)))
    except Exception:
        pass
    return " ".join(meaningful_chunks(chunks))


def pool_values(pool_msg):
    strings = [clean_text(decode_utf8(blob)) for blob in byte_values(pool_msg, 1)]
    rich = [rich_text(blob) for blob in byte_values(pool_msg, 2)]
    nums = [str(v) for v in values(pool_msg, 3)]
    return {4: strings, 6: rich, 2: nums}


def value_from_pool(pool, value_type, ref):
    vals = pool.get(value_type) or []
    return vals[ref] if 0 <= ref < len(vals) else ""


def normalize_link(link):
    link = clean_text(unquote(link)).replace("&amp;", "&").replace("&amp", "&")
    link = link.rstrip("，。；;、)）]】>")
    lower = link.lower()
    for prefix in ("https://magnet:?", "http://magnet:?"):
        if lower.startswith(prefix):
            return "magnet:?" + link[len(prefix) :]
    for prefix in ("https://ed2k://", "http://ed2k://"):
        if lower.startswith(prefix):
            return "ed2k://" + link[len(prefix) :]
    return link


def link_type(link):
    lower = link.lower()
    if lower.startswith("magnet:"):
        return "magnet"
    if lower.startswith("ed2k:"):
        return "ed2k"
    if "/s/" in lower and ("115cdn.com" in lower or "115.com" in lower or "anxia.com" in lower):
        return "share115"
    return "other"


def strip_links(text):
    return clean_text(LINK_RE.sub(" ", text))


def strip_rich_prefix(text):
    return re.sub(r"^[a-z\^_`~|\\/]{1,4}(?=[\u4e00-\u9fff])", "", text)


def normalize_name_text(text):
    text = strip_links(clean_text(text))
    if not text:
        return ""
    parts = []
    for part in text.split(" "):
        part = part.strip(" *<>")
        part = re.sub(r"^(?:[A-Fa-f0-9]{6,8})$", "", part)
        part = strip_rich_prefix(part)
        if not part or part in STYLE_SKIP or re.fullmatch(r"[A-Fa-f0-9]{6,8}", part):
            continue
        if SIZE_RE.match(part) or DATE_RE.match(part):
            continue
        parts.append(part)
    if parts:
        best = max(parts, key=lambda s: (len(s), bool(re.search(r"[\u4e00-\u9fffA-Za-z]", s))))
        if len(best) >= 6:
            return clean_text(best)
    text = re.sub(r"^(?:\*\s*)?(?:[A-Fa-f0-9]{6,8}\s+)+", "", text).strip()
    return clean_text(strip_rich_prefix(text))


def looks_bad_name(text):
    text = clean_text(text)
    if not text or len(text) < 2:
        return True
    if BAD_NAME_RE.search(text) or TYPE_WORDS.match(text):
        return True
    if DATE_RE.match(text) or SIZE_RE.match(text):
        return True
    if re.fullmatch(r"[\d,，.\-_/ ]+", text):
        return True
    return len(text) > 280


def choose_name(row, link_col, sheet_name):
    candidates = []
    for col, value in row.items():
        name = normalize_name_text(value)
        if looks_bad_name(name):
            continue
        score = 0
        if col == link_col:
            score -= 25
        elif col < link_col:
            score += 12
        if len(name) >= 8:
            score += 8
        if any(hint in name for hint in HEADER_HINTS):
            score += 3
        if re.search(r"[\u4e00-\u9fffA-Za-z]", name):
            score += 5
        if re.search(r"(1080p|2160p|BluRay|WEB-DL|REMUX|UHD|x26[45]|DDP|DTS|S\d{2}|第\d+季)", name, re.I):
            score += 6
        if re.search(r"(字幕|分辨率|大小|日期|更新|类型|格式)$", name):
            score -= 12
        candidates.append((score, -abs(col - link_col), -len(name), name))
    if candidates:
        candidates.sort(reverse=True)
        return candidates[0][3]
    for col in sorted(row):
        name = normalize_name_text(row[col])
        if not looks_bad_name(name):
            return name
    return clean_text(sheet_name)


def gunzip_body(data):
    start = data.find(b"\x1f\x8b")
    if start < 0:
        return None
    try:
        return zlib.decompressobj(16 + zlib.MAX_WBITS).decompress(data[start:])
    except Exception:
        return None


def unwrap_json(body):
    text = body.decode("utf-8", "replace").strip()
    match = re.match(r"^[\w.$]+\((.*)\)\s*;?\s*$", text, re.S)
    if match:
        text = match.group(1)
    try:
        return json.loads(text)
    except Exception:
        return None


def iter_related_sheet(obj):
    if isinstance(obj, dict):
        related = obj.get("related_sheet")
        if isinstance(related, str):
            yield related
        for value in obj.values():
            yield from iter_related_sheet(value)
    elif isinstance(obj, list):
        for value in obj:
            yield from iter_related_sheet(value)


def decode_related_sheet(value):
    raw = base64.b64decode(value + "=" * (-len(value) % 4))
    return zlib.decompress(raw)


def extract_blocks(related_sheet):
    decoded = decode_related_sheet(related_sheet)
    root = parse_msg(decoded)
    root_children = child_msgs(root, 1)
    main = root_children[0] if root_children else root
    for cmd in child_msgs(main, 5):
        cmd_types = values(cmd, 1)
        if cmd_types and cmd_types[0] != 18:
            continue
        for block in child_msgs(cmd, 19):
            yield block


def rows_from_block(block):
    info_msgs = child_msgs(block, 3)
    if not info_msgs:
        return None, {}
    sheet_ids = byte_values(info_msgs[0], 1)
    sheet_id = decode_utf8(sheet_ids[0]) if sheet_ids else ""
    pool_msgs = child_msgs(block, 5)
    if not pool_msgs:
        return sheet_id, {}
    pool = pool_values(pool_msgs[0])
    rows = collections.defaultdict(dict)
    for cell_pos in child_msgs(block, 6):
        row_vals = values(cell_pos, 1)
        col_vals = values(cell_pos, 2)
        row_idx = row_vals[0] if row_vals else 0
        col_idx = col_vals[0] if col_vals else 0
        cell_datas = child_msgs(cell_pos, 3)
        if not cell_datas:
            continue
        cell_data = cell_datas[0]
        type_vals = values(cell_data, 1)
        value_type = type_vals[0] if type_vals else None
        ref = cell_ref_index(cell_data)
        value = clean_text(value_from_pool(pool, value_type, ref))
        if value:
            rows[row_idx][col_idx] = value
    return sheet_id, rows


def sheet_names_from_obj(obj):
    names = {}
    try:
        headers = obj["clientVars"]["collab_client_vars"]["header"]
    except Exception:
        return names
    for header in headers:
        for item in header.get("d", []):
            if item.get("hidden"):
                continue
            sheet_id = item.get("id")
            name = item.get("name")
            if sheet_id and name:
                names[sheet_id] = name
    return names


def discover_chrome_cache_dirs():
    roots = []
    base = Path.home() / "Library" / "Caches" / "Google" / "Chrome"
    roots.extend(base.glob("*/Cache/Cache_Data"))
    roots.extend(base.glob("*/Code Cache/js"))
    return [p for p in roots if p.is_dir()]


def cache_url(data):
    match = URL_RE.search(data)
    if not match:
        return ""
    return match.group(0).decode("utf-8", "replace")


def iter_cache_objects(cache_dirs, doc_id, pad_mark, since_hours):
    cutoff = 0
    if since_hours and since_hours > 0:
        cutoff = time.time() - since_hours * 3600
    paths = []
    for cache_dir in cache_dirs:
        for path in cache_dir.iterdir():
            if not path.is_file():
                continue
            try:
                stat = path.stat()
            except OSError:
                continue
            if cutoff and stat.st_mtime < cutoff:
                continue
            try:
                data = path.read_bytes()
            except OSError:
                continue
            if b"docs.qq.com/dop-api/" not in data:
                continue
            if doc_id.encode() not in data and pad_mark.encode() not in data:
                continue
            url = cache_url(data)
            if not url:
                continue
            paths.append((stat.st_mtime, path, data))

    for _, path, data in sorted(paths):
        body = gunzip_body(data)
        if not body:
            continue
        obj = unwrap_json(body)
        if obj:
            yield path, obj


def extract_records(cache_dirs, doc_id, pad_mark, since_hours):
    stats = {
        "cache_objects": 0,
        "related_blocks": 0,
        "decoded_blocks": 0,
        "errors": 0,
        "error_samples": [],
        "sheet_names": {},
        "per_sheet": collections.Counter(),
        "types": collections.Counter(),
        "no_link_rows": collections.Counter(),
    }
    objects = list(iter_cache_objects(cache_dirs, doc_id, pad_mark, since_hours))
    for _, obj in objects:
        stats["sheet_names"].update(sheet_names_from_obj(obj))

    records = set()
    for path, obj in objects:
        related_values = list(iter_related_sheet(obj))
        if not related_values:
            continue
        stats["cache_objects"] += 1
        for related in related_values:
            stats["related_blocks"] += 1
            try:
                for block in extract_blocks(related):
                    stats["decoded_blocks"] += 1
                    sheet_id, rows = rows_from_block(block)
                    if not sheet_id or not rows:
                        continue
                    sheet = stats["sheet_names"].get(sheet_id, sheet_id)
                    for row in rows.values():
                        links = []
                        for col, value in row.items():
                            for match in LINK_RE.finditer(value):
                                link = normalize_link(match.group(0))
                                lt = link_type(link)
                                if lt in ("share115", "magnet", "ed2k"):
                                    links.append((col, link, lt))
                        if not links:
                            stats["no_link_rows"][sheet] += 1
                            continue
                        seen_links = set()
                        for col, link, lt in links:
                            if link in seen_links:
                                continue
                            seen_links.add(link)
                            name = choose_name(row, col, sheet)
                            is_pkg = 1 if PKG_RE.search(name + " " + sheet) else 0
                            rec = (name, sheet, link, is_pkg, lt)
                            if rec not in records:
                                records.add(rec)
                                stats["per_sheet"][sheet] += 1
                                stats["types"][lt] += 1
            except Exception as exc:
                stats["errors"] += 1
                if len(stats["error_samples"]) < 5:
                    stats["error_samples"].append((path.name, str(exc)[:200]))
    return sorted(records, key=lambda r: (r[1], r[0], r[2])), stats


def write_db(records, db_path, min_rows, dry_run):
    type_counts = collections.Counter(r[4] for r in records)
    sheet_count = len({r[1] for r in records})
    package_count = sum(1 for r in records if r[3])
    if len(records) < min_rows:
        raise SystemExit("refuse to replace DB: extracted only %d rows (< %d)" % (len(records), min_rows))
    if dry_run:
        return None, {"rows": len(records), "sheets": sheet_count, "packages": package_count, "types": type_counts}

    db_path = Path(db_path)
    backup = db_path.with_name("%s.bak-%s" % (db_path.name, time.strftime("%Y%m%d-%H%M%S")))
    if db_path.exists():
        shutil.copy2(db_path, backup)
    tmp = db_path.with_name("%s.tmp-%s" % (db_path.name, time.strftime("%Y%m%d-%H%M%S")))
    if tmp.exists():
        tmp.unlink()
    con = sqlite3.connect(str(tmp))
    try:
        con.execute("PRAGMA journal_mode=OFF")
        con.execute("PRAGMA synchronous=OFF")
        con.execute("CREATE TABLE catalog(name TEXT, sheet TEXT, link TEXT, is_pkg INT, link_type TEXT)")
        con.executemany("INSERT INTO catalog(name,sheet,link,is_pkg,link_type) VALUES (?,?,?,?,?)", records)
        con.commit()
    finally:
        con.close()
    os.replace(tmp, db_path)
    return backup, {"rows": len(records), "sheets": sheet_count, "packages": package_count, "types": type_counts}


def print_stats(summary, stats):
    print("records:", summary["rows"])
    print("sheets:", summary["sheets"])
    print("packages:", summary["packages"])
    print("types:", dict(summary["types"].most_common()))
    print("cache_objects:", stats["cache_objects"])
    print("related_blocks:", stats["related_blocks"])
    print("decoded_blocks:", stats["decoded_blocks"])
    print("sheet_names:", len(stats["sheet_names"]))
    print("errors:", stats["errors"])
    print("top_sheets:")
    for sheet, count in stats["per_sheet"].most_common(25):
        print("  %6d  %s" % (count, sheet))
    print("top_no_link_rows:")
    for sheet, count in stats["no_link_rows"].most_common(12):
        print("  %6d  %s" % (count, sheet))
    if stats["error_samples"]:
        print("error_samples:", stats["error_samples"])


def parse_args(argv):
    parser = argparse.ArgumentParser(description="Update catalog_115.db from Tencent Docs Chrome cache.")
    parser.add_argument("--db", default=str(DEFAULT_DB), help="target catalog sqlite db")
    parser.add_argument("--doc-id", default=DEFAULT_DOC_ID, help="Tencent Docs public id")
    parser.add_argument("--pad-mark", default=DEFAULT_PAD_MARK, help="stable pad id substring seen in docs.qq.com API URLs")
    parser.add_argument("--cache-dir", action="append", help="Chrome Cache_Data dir; can be repeated")
    parser.add_argument(
        "--since-hours",
        type=float,
        default=DEFAULT_SINCE_HOURS,
        help="only use Chrome cache files modified in the last N hours; 0 means all cache",
    )
    parser.add_argument("--min-rows", type=int, default=DEFAULT_MIN_ROWS, help="refuse replacement below this row count")
    parser.add_argument("--dry-run", action="store_true", help="parse and print stats, do not replace DB")
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv or sys.argv[1:])
    cache_dirs = [Path(p).expanduser() for p in args.cache_dir] if args.cache_dir else discover_chrome_cache_dirs()
    cache_dirs = [p for p in cache_dirs if p.is_dir()]
    if not cache_dirs:
        raise SystemExit("no Chrome cache dirs found; pass --cache-dir")
    records, stats = extract_records(cache_dirs, args.doc_id, args.pad_mark, args.since_hours)
    backup, summary = write_db(records, Path(args.db), args.min_rows, args.dry_run)
    print_stats(summary, stats)
    if args.dry_run:
        print("dry_run: DB not modified")
    else:
        print("db_replaced:", args.db)
        print("backup:", backup)


if __name__ == "__main__":
    main()
