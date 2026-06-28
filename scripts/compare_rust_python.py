#!/usr/bin/env python3
"""灰度对比 Python 旧版和 Rust 新版的基础接口。

默认只测无需登录的 health 与 Rust v2 骨架接口；如传 token/cookie，可扩展对比受保护接口。
"""
import argparse
import json
import sys
import urllib.error
import urllib.request


def fetch_json(base, path, token=None, timeout=5):
    req = urllib.request.Request(base.rstrip("/") + path)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read().decode("utf-8", "replace")
            return {"ok": True, "status": resp.status, "json": json.loads(body)}
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        try:
            payload = json.loads(body)
        except Exception:
            payload = body
        return {"ok": False, "status": e.code, "json": payload}
    except Exception as e:
        return {"ok": False, "status": None, "err": str(e)}


def check(label, result, expect_ok=True):
    ok = bool(result.get("ok")) is expect_ok
    mark = "OK" if ok else "FAIL"
    print("[%s] %s status=%s" % (mark, label, result.get("status")))
    if not ok:
        print(json.dumps(result, ensure_ascii=False, indent=2))
    return ok


def skip(label, reason):
    print("[SKIP] %s %s" % (label, reason))
    return True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--python", default="http://127.0.0.1:8097")
    ap.add_argument("--rust", default="http://127.0.0.1:8098")
    ap.add_argument("--rust-token", default="")
    ap.add_argument("--python-token", default="")
    args = ap.parse_args()

    checks = []
    py_health = fetch_json(args.python, "/health", args.python_token)
    rs_health = fetch_json(args.rust, "/health", args.rust_token)
    checks.append(check("python /health", py_health))
    checks.append(check("rust /health", rs_health))

    for path in ("/api/v2/openapi.json",):
        checks.append(check("rust " + path, fetch_json(args.rust, path, args.rust_token)))

    protected_paths = (
        "/api/v2/tasks?limit=5",
        "/api/v2/catalog/stats",
        "/api/v2/system/summary",
    )
    if args.rust_token:
        for path in protected_paths:
            checks.append(check("rust " + path, fetch_json(args.rust, path, args.rust_token)))
    else:
        for path in protected_paths:
            checks.append(skip("rust " + path, "(requires --rust-token)"))

    if py_health.get("json") and rs_health.get("json"):
        print("\nhealth summary")
        print("python:", json.dumps(py_health["json"], ensure_ascii=False, sort_keys=True))
        print("rust:  ", json.dumps(rs_health["json"], ensure_ascii=False, sort_keys=True))

    if not all(checks):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
