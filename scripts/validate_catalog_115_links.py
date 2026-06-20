#!/usr/bin/env python3
"""Validate 115 share links from catalog_115.db.

The catalog can contain more than 100k 115 share rows, so this script is
deliberately slow, resumable, and side-effect free:

  python3 scripts/validate_catalog_115_links.py --limit 20
  python3 scripts/validate_catalog_115_links.py --sleep 1.0

Results are written to catalog_115_validation.db by default. The source
catalog_115.db is only read.
"""
import argparse
import json
import os
import re
import sqlite3
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CATALOG_DB = ROOT / "catalog_115.db"
DEFAULT_OUT_DB = ROOT / "catalog_115_validation.db"
DEFAULT_CONFIG = ROOT / "config.json"
DEFAULT_SLEEP_SEC = 1.0

C115_API = "https://webapi.115.com"
C115_UA = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36"
)


def parse_115_url(url, pwd=None):
    """Return (share_code, receive_code) from a 115 share URL or plain pair."""
    url = (url or "").strip()
    share = None
    rc = None
    m = re.search(r"/s/([0-9a-zA-Z]+)", url)
    if m:
        share = m.group(1)
    m = re.search(r"[?&](?:password|pwd|pickcode)=([^&#\s]+)", url)
    if m:
        rc = urllib.parse.unquote(m.group(1))
    if not share:
        parts = re.split(r"[\s,]+", url)
        if parts and re.match(r"^[0-9a-zA-Z]+$", parts[0]):
            share = parts[0]
            if len(parts) > 1 and re.match(r"^[0-9a-zA-Z]+$", parts[1]):
                rc = parts[1]
    if pwd:
        rc = pwd.strip()
    return share, rc or ""


def load_cookie(args):
    if args.cookie:
        return args.cookie
    env_cookie = os.environ.get("C115_COOKIE") or os.environ.get("EMBY_MANAGER_C115_COOKIE")
    if env_cookie:
        return env_cookie
    if args.cookie_file:
        raw = Path(args.cookie_file).read_text(encoding="utf-8").strip()
        if raw.startswith("{"):
            return json.loads(raw).get("c115_cookie", "") or ""
        return raw
    if DEFAULT_CONFIG.exists():
        try:
            return json.loads(DEFAULT_CONFIG.read_text(encoding="utf-8")).get("c115_cookie", "") or ""
        except Exception:
            return ""
    return ""


def init_out_db(path):
    con = sqlite3.connect(path)
    con.execute("PRAGMA journal_mode=WAL")
    con.execute(
        """
        CREATE TABLE IF NOT EXISTS share_checks (
            share_code TEXT NOT NULL,
            receive_code TEXT NOT NULL DEFAULT '',
            ok INTEGER NOT NULL,
            status TEXT NOT NULL,
            err TEXT,
            share_title TEXT,
            file_count INTEGER,
            raw_msg TEXT,
            checked_at INTEGER NOT NULL,
            PRIMARY KEY (share_code, receive_code)
        )
        """
    )
    con.execute(
        """
        CREATE TABLE IF NOT EXISTS runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            source_db TEXT NOT NULL,
            checked INTEGER NOT NULL DEFAULT 0,
            skipped INTEGER NOT NULL DEFAULT 0,
            ok_count INTEGER NOT NULL DEFAULT 0,
            bad_count INTEGER NOT NULL DEFAULT 0,
            err TEXT
        )
        """
    )
    con.commit()
    return con


def catalog_has_link_type(con):
    try:
        return any(c[1] == "link_type" for c in con.execute("PRAGMA table_info(catalog)").fetchall())
    except Exception:
        return False


def iter_catalog_shares(catalog_db):
    """Yield unique (share_code, receive_code) pairs from catalog rows."""
    seen = set()
    with sqlite3.connect(f"file:{catalog_db}?mode=ro", uri=True) as con:
        has_type = catalog_has_link_type(con)
        if has_type:
            sql = "SELECT link FROM catalog WHERE link_type='share115'"
            rows = con.execute(sql)
        else:
            sql = (
                "SELECT link FROM catalog WHERE link LIKE '%/s/%' "
                "OR link LIKE '%115.com%' OR link LIKE '%115cdn.com%' OR link LIKE '%anxia.com%'"
            )
            rows = con.execute(sql)
        for (link,) in rows:
            share, rc = parse_115_url(link)
            if not share:
                continue
            key = (share, rc or "")
            if key in seen:
                continue
            seen.add(key)
            yield key


def recently_checked(out_con, share, rc, recheck_days):
    if recheck_days <= 0:
        return False
    cutoff = int(time.time() - recheck_days * 86400)
    row = out_con.execute(
        "SELECT checked_at FROM share_checks WHERE share_code=? AND receive_code=?",
        (share, rc),
    ).fetchone()
    return bool(row and row[0] >= cutoff)


def request_snap(cookie, share, rc, timeout=30):
    params = {
        "share_code": share,
        "receive_code": rc or "",
        "cid": 0,
        "offset": 0,
        "limit": 1,
    }
    url = C115_API + "/share/snap?" + urllib.parse.urlencode(params)
    headers = {
        "User-Agent": C115_UA,
        "Cookie": cookie,
        "Referer": "https://115.com/",
        "Accept": "application/json, text/plain, */*",
    }
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", "ignore")
            return json.loads(raw)
    except urllib.error.HTTPError as e:
        body = ""
        try:
            body = e.read().decode("utf-8", "ignore")[:300]
        except Exception:
            pass
        return {"state": False, "error": f"HTTP {e.code}: {body}"}
    except Exception as e:
        return {"state": False, "error": str(e)}


def classify_snap(resp):
    ok = bool(resp.get("state"))
    data = resp.get("data") or {}
    shareinfo = data.get("shareinfo") or {}
    title = shareinfo.get("share_title") or shareinfo.get("title") or ""
    file_count = shareinfo.get("file_count")
    if file_count is None:
        file_count = len(data.get("list") or [])
    err = resp.get("error") or resp.get("msg") or resp.get("errno") or ""
    status = "ok" if ok else "invalid"
    if not ok and err:
        lowered = str(err).lower()
        if "http 403" in lowered or "http 405" in lowered or "http 429" in lowered:
            status = "network_error"
        elif "cookie" in lowered or "login" in lowered or "登录" in str(err):
            status = "auth_error"
        elif "timeout" in lowered or "timed out" in lowered:
            status = "network_error"
    return {
        "ok": 1 if ok else 0,
        "status": status,
        "err": "" if ok else str(err)[:500],
        "share_title": str(title)[:500],
        "file_count": int(file_count or 0),
        "raw_msg": json.dumps(resp, ensure_ascii=False)[:1000],
    }


def save_check(con, share, rc, result):
    con.execute(
        """
        INSERT INTO share_checks
          (share_code, receive_code, ok, status, err, share_title, file_count, raw_msg, checked_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(share_code, receive_code) DO UPDATE SET
          ok=excluded.ok,
          status=excluded.status,
          err=excluded.err,
          share_title=excluded.share_title,
          file_count=excluded.file_count,
          raw_msg=excluded.raw_msg,
          checked_at=excluded.checked_at
        """,
        (
            share,
            rc,
            result["ok"],
            result["status"],
            result["err"],
            result["share_title"],
            result["file_count"],
            result["raw_msg"],
            int(time.time()),
        ),
    )


def main(argv=None):
    p = argparse.ArgumentParser(description="Validate share115 links in catalog_115.db.")
    p.add_argument("--db", default=str(DEFAULT_CATALOG_DB), help="source catalog db")
    p.add_argument("--out-db", default=str(DEFAULT_OUT_DB), help="validation result db")
    p.add_argument("--cookie", default="", help="115 cookie string")
    p.add_argument("--cookie-file", default="", help="plain cookie file or JSON config containing c115_cookie")
    p.add_argument("--cookie-stdin", action="store_true", help="read one cookie line from stdin")
    p.add_argument("--no-cookie", action="store_true", help="validate public shares without a 115 cookie")
    p.add_argument("--limit", type=int, default=0, help="max number of links to check this run")
    p.add_argument("--sleep", type=float, default=DEFAULT_SLEEP_SEC, help="seconds between requests")
    p.add_argument("--workers", type=int, default=1, help="parallel request workers; keep small to avoid 115 throttling")
    p.add_argument("--recheck-days", type=float, default=30, help="skip links checked within N days; 0 disables skip")
    p.add_argument("--timeout", type=int, default=30, help="HTTP timeout seconds")
    p.add_argument("--dry-run", action="store_true", help="only count unique share links")
    args = p.parse_args(argv)

    catalog_db = Path(args.db)
    if not catalog_db.exists():
        print(f"source db not found: {catalog_db}", file=sys.stderr)
        return 2

    links = iter_catalog_shares(catalog_db)
    if args.dry_run:
        total = sum(1 for _ in links)
        print(f"unique share115 links: {total}")
        return 0

    if args.no_cookie:
        cookie = ""
    elif args.cookie_stdin:
        cookie = sys.stdin.readline().strip()
    else:
        cookie = load_cookie(args)
    if not cookie and not args.no_cookie:
        print("missing 115 cookie: pass --cookie, --cookie-file, or C115_COOKIE", file=sys.stderr)
        return 2

    out_con = init_out_db(args.out_db)
    started = int(time.time())
    cur = out_con.execute("INSERT INTO runs(started_at, source_db) VALUES (?, ?)", (started, str(catalog_db)))
    run_id = cur.lastrowid
    out_con.commit()

    checked = skipped = ok_count = bad_count = 0
    err = None
    workers = max(1, min(16, int(args.workers or 1)))

    def check_one(share, rc):
        resp = request_snap(cookie, share, rc, timeout=args.timeout)
        result = classify_snap(resp)
        if args.sleep > 0:
            time.sleep(args.sleep)
        return share, rc, result

    def consume_result(item):
        nonlocal checked, ok_count, bad_count
        share, rc, result = item
        save_check(out_con, share, rc, result)
        out_con.commit()
        checked += 1
        if result["ok"]:
            ok_count += 1
        else:
            bad_count += 1
        print(
            f"[{checked}] {result['status']} {share} "
            f"{('title=' + result['share_title']) if result['share_title'] else result['err']}",
            flush=True,
        )

    try:
        if workers == 1:
            for share, rc in links:
                if args.limit and checked >= args.limit:
                    break
                if recently_checked(out_con, share, rc, args.recheck_days):
                    skipped += 1
                    continue
                consume_result(check_one(share, rc))
        else:
            pending = set()
            with ThreadPoolExecutor(max_workers=workers) as pool:
                for share, rc in links:
                    if args.limit and checked + len(pending) >= args.limit:
                        break
                    if recently_checked(out_con, share, rc, args.recheck_days):
                        skipped += 1
                        continue
                    pending.add(pool.submit(check_one, share, rc))
                    while len(pending) >= workers:
                        done, pending = wait(pending, return_when=FIRST_COMPLETED)
                        for fut in done:
                            consume_result(fut.result())
                while pending:
                    done, pending = wait(pending, return_when=FIRST_COMPLETED)
                    for fut in done:
                        consume_result(fut.result())
    except KeyboardInterrupt:
        err = "interrupted"
        print("interrupted; progress has been saved", file=sys.stderr)
    except Exception as e:
        err = str(e)
        raise
    finally:
        out_con.execute(
            """
            UPDATE runs
               SET ended_at=?, checked=?, skipped=?, ok_count=?, bad_count=?, err=?
             WHERE id=?
            """,
            (int(time.time()), checked, skipped, ok_count, bad_count, err, run_id),
        )
        out_con.commit()

    print(f"done: checked={checked} skipped={skipped} ok={ok_count} bad={bad_count} out={args.out_db}")
    return 1 if err else 0


if __name__ == "__main__":
    raise SystemExit(main())
