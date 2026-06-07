"""HTTP handler 端到端集成测试。

真起 ThreadingHTTPServer + 真打 HTTP 请求,不 mock socket 层。覆盖:
- /health 公开
- /api/* auth gate
- /api/login 成功/错密码/限流/默认空密码拒登
- cookie 流(Set-Cookie + Cookie 回带 + /api/me + /api/logout 失效)
- CSRF 校验(缺/错/对)
- 安全头(X-Content-Type-Options / X-Frame-Options / Referrer-Policy / CSP)
- path traversal(_safe_under 拒非法库名)
- /api/task 未知 tid

⚠️ 测试间通过 LOGIN_FAIL.clear() 隔离限流计数;不要并发跑。
⚠️ 启动前重定向 lib.config.HERE → TMPDIR,免污染真 config.json / logs/。
"""
import os, sys, json, threading, time, unittest, tempfile, shutil
from http.client import HTTPConnection
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

# 关键:在 import app 之前,把 config 切到隔离的临时目录,免污染真 config.json
TMPDIR = tempfile.mkdtemp(prefix="embymgr_test_")

import lib.config
lib.config.HERE = TMPDIR
lib.config.CONFIG_FILE = os.path.join(TMPDIR, "config.json")
lib.config.load_cfg()  # reload 进 CFG(空 = defaults)

import app
from lib.config import CFG
from lib.auth import _hash_password, TOKENS, LOGIN_FAIL
from http.server import ThreadingHTTPServer

SERVER = None
SERVER_PORT = None
SERVER_THREAD = None
TEST_PW = "test_pw_12345!"


def setUpModule():
    global SERVER, SERVER_PORT, SERVER_THREAD
    # 设密码(用强密码避开 WEAK_PWS)
    CFG["password_hash"] = _hash_password(TEST_PW)
    CFG["host"] = "127.0.0.1"
    CFG["schema_version"] = 99  # 跳过 migration
    # 写一个最小 VERSION 文件,/health 才能读到 version
    try:
        with open(os.path.join(TMPDIR, "VERSION"), "w") as f:
            f.write("test-0.0.0")
    except Exception:
        pass
    SERVER = ThreadingHTTPServer(("127.0.0.1", 0), app.H)
    SERVER_PORT = SERVER.server_port
    SERVER_THREAD = threading.Thread(target=SERVER.serve_forever, daemon=True)
    SERVER_THREAD.start()
    time.sleep(0.1)


def tearDownModule():
    global SERVER
    if SERVER:
        SERVER.shutdown()
        SERVER.server_close()
    try:
        shutil.rmtree(TMPDIR)
    except Exception:
        pass


def req(method, path, body=None, cookie=None, csrf=None, raw=False, timeout=10):
    """直接走 http.client.HTTPConnection 拿原始响应(headers + body)。"""
    c = HTTPConnection("127.0.0.1", SERVER_PORT, timeout=timeout)
    headers = {"Content-Type": "application/json"}
    if cookie:
        headers["Cookie"] = cookie
    if csrf:
        headers["X-CSRF-Token"] = csrf
    data = json.dumps(body).encode("utf-8") if body is not None else None
    c.request(method, path, data, headers)
    r = c.getresponse()
    body_bytes = r.read()
    c.close()
    # 多个同名 header(如 Set-Cookie)dict() 会保留最后一个,够测当前场景
    if raw:
        return r.status, dict(r.getheaders()), body_bytes
    try:
        parsed = json.loads(body_bytes.decode("utf-8", "ignore")) if body_bytes else {}
    except Exception:
        parsed = {}
    return r.status, dict(r.getheaders()), parsed


def parse_set_cookie(headers):
    """提取 Set-Cookie 中 emby_tok=... 的值"""
    sc = headers.get("Set-Cookie", "")
    for chunk in sc.split(";"):
        chunk = chunk.strip()
        if chunk.startswith("emby_tok="):
            return chunk[len("emby_tok="):]
    return None


def clear_login_fail():
    """清限流计数(测试间相互独立)"""
    with __import__("lib.auth", fromlist=["LOGIN_FAIL_LOCK"]).LOGIN_FAIL_LOCK:
        LOGIN_FAIL.clear()


def login_get_token_csrf():
    """登录助手:返 (token, csrf);失败抛"""
    clear_login_fail()
    s, h, b = req("POST", "/api/login", {"pw": TEST_PW})
    if s != 200:
        raise AssertionError("login failed: %d %r" % (s, b))
    return parse_set_cookie(h), b.get("csrf")


# ============================================================
# 1) /health 公开
# ============================================================
class HealthTests(unittest.TestCase):
    def test_health_public_no_auth(self):
        s, h, b = req("GET", "/health")
        self.assertEqual(s, 200)
        for k in ("status", "version", "uptime", "emby_online", "c115_cookie_set", "cd_mounted"):
            self.assertIn(k, b, "/health 缺字段 %s" % k)
        self.assertEqual(b["status"], "ok")
        self.assertIsInstance(b["uptime"], int)
        self.assertIsInstance(b["emby_online"], bool)


# ============================================================
# 2) /api/* 必须 auth
# ============================================================
class AuthGateTests(unittest.TestCase):
    def test_api_libraries_requires_auth(self):
        s, h, b = req("GET", "/api/libraries")
        self.assertEqual(s, 401)
        self.assertEqual(b.get("err"), "未登录")

    def test_api_system_requires_auth(self):
        s, h, b = req("GET", "/api/system")
        self.assertEqual(s, 401)

    def test_api_unknown_post_requires_auth(self):
        # 非 login/logout 的 POST 也得先 auth
        s, h, b = req("POST", "/api/scan", {"lib": "x"})
        self.assertEqual(s, 401)


# ============================================================
# 3) /api/login 成功流程
# ============================================================
class LoginSuccessTests(unittest.TestCase):
    def setUp(self):
        clear_login_fail()

    def test_correct_password_sets_secure_cookie(self):
        s, h, b = req("POST", "/api/login", {"pw": TEST_PW})
        self.assertEqual(s, 200)
        self.assertTrue(b.get("ok"))
        self.assertIsInstance(b.get("csrf"), str)
        self.assertGreater(len(b["csrf"]), 8)
        sc = h.get("Set-Cookie", "")
        self.assertIn("emby_tok=", sc)
        self.assertIn("HttpOnly", sc)
        self.assertIn("SameSite=Strict", sc)
        self.assertIn("Path=/", sc)

    def test_token_can_then_call_api(self):
        # 拿到 cookie 后调 /api/libraries,期望 auth 层放行(emby 离线业务可能 fail,但应是 200)
        tok, csrf = login_get_token_csrf()
        s, h, b = req("GET", "/api/libraries", cookie="emby_tok=" + tok)
        # auth 通过了 → 不会 401。emby 离线时返 200 + {emby: {online:false}, libs:[], excluded:[...]}
        self.assertEqual(s, 200, "auth 应放行,实际 %d %r" % (s, b))


# ============================================================
# 4) 错密码 → 403,达到阈值 → 429
# ============================================================
class LoginFailTests(unittest.TestCase):
    def setUp(self):
        clear_login_fail()

    def test_wrong_password_403(self):
        s, h, b = req("POST", "/api/login", {"pw": "definitely-wrong-pw"})
        self.assertEqual(s, 403)
        self.assertIn("err", b)

    def test_rate_limit_after_5_failures(self):
        # 1..5 次错应 403,第 6 次起 429
        for i in range(5):
            s, h, b = req("POST", "/api/login", {"pw": "wrong_%d" % i})
            self.assertEqual(s, 403, "第 %d 次错密码,应 403,实际 %d" % (i + 1, s))
        # 第 6 次:不论密码对错,limit 已超
        s, h, b = req("POST", "/api/login", {"pw": "wrong_6"})
        self.assertEqual(s, 429, "第 6 次应被限流,实际 %d %r" % (s, b))
        # 即使送对密码也限流(_login_allowed 先判)
        s, h, b = req("POST", "/api/login", {"pw": TEST_PW})
        self.assertEqual(s, 429, "限流期间正确密码也应 429")


# ============================================================
# 5) 默认密码空(password_hash 未设)→ 任意密码 403
# ============================================================
class EmptyPasswordHashTests(unittest.TestCase):
    def setUp(self):
        clear_login_fail()
        # 临时清空 password_hash
        self._saved = CFG.get("password_hash", "")
        CFG["password_hash"] = ""

    def tearDown(self):
        CFG["password_hash"] = self._saved

    def test_empty_pw_rejected(self):
        s, h, b = req("POST", "/api/login", {"pw": ""})
        self.assertEqual(s, 403)

    def test_any_pw_rejected_when_no_hash(self):
        s, h, b = req("POST", "/api/login", {"pw": "anything"})
        self.assertEqual(s, 403)

    def test_no_fallback_to_plaintext_password_field(self):
        # 模拟旧 schema 残留 plain password 字段 → 不应被接受(_verify_password 只看 hash)
        CFG["password"] = "plaintext_legacy"  # 故意 contaminate
        try:
            s, h, b = req("POST", "/api/login", {"pw": "plaintext_legacy"})
            self.assertEqual(s, 403, "不能 fallback 到旧明文")
        finally:
            CFG.pop("password", None)


# ============================================================
# 6) /api/me
# ============================================================
class MeEndpointTests(unittest.TestCase):
    def test_me_with_cookie_returns_csrf(self):
        tok, csrf = login_get_token_csrf()
        s, h, b = req("GET", "/api/me", cookie="emby_tok=" + tok)
        self.assertEqual(s, 200)
        self.assertEqual(b.get("csrf"), csrf)

    def test_me_without_cookie_401(self):
        s, h, b = req("GET", "/api/me")
        self.assertEqual(s, 401)

    def test_me_with_bogus_cookie_401(self):
        s, h, b = req("GET", "/api/me", cookie="emby_tok=not-a-real-token")
        self.assertEqual(s, 401)


# ============================================================
# 7) CSRF 校验
# ============================================================
class CsrfTests(unittest.TestCase):
    def test_post_without_csrf_403(self):
        tok, _ = login_get_token_csrf()
        # POST 不带 X-CSRF-Token
        s, h, b = req("POST", "/api/createlib", {"name": "x_csrf_test", "ctype": "movies"},
                      cookie="emby_tok=" + tok)
        self.assertEqual(s, 403)
        self.assertIn("CSRF", b.get("err", ""))

    def test_post_wrong_csrf_403(self):
        tok, _ = login_get_token_csrf()
        s, h, b = req("POST", "/api/createlib", {"name": "x_csrf_test", "ctype": "movies"},
                      cookie="emby_tok=" + tok, csrf="bogus-csrf-token")
        self.assertEqual(s, 403)
        self.assertIn("CSRF", b.get("err", ""))

    def test_post_correct_csrf_passes_csrf_layer(self):
        tok, csrf = login_get_token_csrf()
        # csrf 对 → 应能过 csrf 层,业务层可能 fail(emby 离线/createlib 试图 mkdir 不存在的 /volume1/...)
        # 关键是错误信息里不应含 "CSRF"
        s, h, b = req("POST", "/api/createlib",
                      {"name": "test_csrf_layer_pass_%d" % int(time.time()), "ctype": "movies"},
                      cookie="emby_tok=" + tok, csrf=csrf)
        # 不是 csrf 错误
        self.assertNotIn("CSRF", b.get("err", "") or "", "csrf 应通过,实际 err: %r" % b.get("err"))
        # 状态不该是 403(csrf 失败码)
        self.assertNotEqual(s, 403)


# ============================================================
# 8) /api/logout 失效 cookie
# ============================================================
class LogoutTests(unittest.TestCase):
    def test_logout_invalidates_token(self):
        tok, csrf = login_get_token_csrf()
        # logout 是 POST → 需要 csrf
        s, h, b = req("POST", "/api/logout", cookie="emby_tok=" + tok, csrf=csrf)
        self.assertEqual(s, 200)
        self.assertTrue(b.get("ok"))
        # 用同 token 再调 /api/me 应 401
        s, h, b = req("GET", "/api/me", cookie="emby_tok=" + tok)
        self.assertEqual(s, 401, "logout 后 cookie 应失效,实际 %d %r" % (s, b))


# ============================================================
# 9) 安全头(每个响应都有)
# ============================================================
class SecurityHeaderTests(unittest.TestCase):
    def _assert_security_headers(self, h):
        self.assertEqual(h.get("X-Content-Type-Options"), "nosniff")
        self.assertEqual(h.get("X-Frame-Options"), "DENY")
        self.assertEqual(h.get("Referrer-Policy"), "no-referrer")
        csp = h.get("Content-Security-Policy", "")
        self.assertIn("default-src 'self'", csp, "CSP 缺 default-src 'self'")
        self.assertIn("frame-ancestors 'none'", csp, "CSP 缺 frame-ancestors 'none'")

    def test_health_has_security_headers(self):
        s, h, b = req("GET", "/health")
        self._assert_security_headers(h)

    def test_login_response_has_security_headers(self):
        clear_login_fail()
        s, h, b = req("POST", "/api/login", {"pw": "wrong"})
        self._assert_security_headers(h)

    def test_api_401_has_security_headers(self):
        s, h, b = req("GET", "/api/libraries")
        self._assert_security_headers(h)


# ============================================================
# 10) path traversal:createlib 应拒
# ============================================================
class PathTraversalTests(unittest.TestCase):
    def test_createlib_rejects_traversal(self):
        tok, csrf = login_get_token_csrf()
        s, h, b = req("POST", "/api/createlib", {"name": "../etc", "ctype": "movies"},
                      cookie="emby_tok=" + tok, csrf=csrf)
        # 期望:400 + 错误信息含 "非法"。
        # ⚠️ 现状(2026-05-28):create_library 自己 try/except 了 _safe_under 的 ValueError,
        # 把错误包成 {"err": "库名非法: ..."} 然后正常 return 200。
        # 这条会 FAIL,设计 bug:应让 ValueError 冒上去由 do_POST 的 except ValueError 转 400,
        # 或 create_library 自己也返 400(用 AppError(.., status=400))。
        self.assertEqual(s, 400, "path traversal 应 400,实际 %d %r" % (s, b))
        self.assertIn("非法", b.get("err", ""))


# ============================================================
# 11) /api/task?tid=不存在
# ============================================================
class TaskTests(unittest.TestCase):
    def test_unknown_tid_returns_404(self):
        tok, _ = login_get_token_csrf()
        s, h, b = req("GET", "/api/task?tid=nonexistent_tid_xxx", cookie="emby_tok=" + tok)
        self.assertEqual(s, 404)
        self.assertIn("err", b)


# ============================================================
# 12) CD2 webhook:免登录但密钥保护,绝不走 _auth/_csrf
# ============================================================
class Cd2WebhookTests(unittest.TestCase):
    SECRET = "test_webhook_secret_abc123"

    def setUp(self):
        self._saved_secret = CFG.get("cd2_webhook_secret")
        self._saved_enabled = CFG.get("auto_strm_enabled")
        CFG["cd2_webhook_secret"] = self.SECRET
        CFG["auto_strm_enabled"] = True

    def tearDown(self):
        if self._saved_secret is None: CFG.pop("cd2_webhook_secret", None)
        else: CFG["cd2_webhook_secret"] = self._saved_secret
        if self._saved_enabled is None: CFG.pop("auto_strm_enabled", None)
        else: CFG["auto_strm_enabled"] = self._saved_enabled

    def _payload(self):
        return {"data": [{"action": "create", "is_dir": False,
                          "source_file": "/CloudNAS/CloudDrive/电影/片 (2024)/a.mkv"}]}

    def test_no_secret_403(self):
        # 不带 key,且不带任何 cookie/csrf → 直接 403(密钥层,不是 401 未登录)
        s, h, b = req("POST", "/api/cd2/webhook", self._payload())
        self.assertEqual(s, 403)
        self.assertEqual(b.get("err"), "forbidden")

    def test_wrong_secret_403(self):
        s, h, b = req("POST", "/api/cd2/webhook?key=WRONG", self._payload())
        self.assertEqual(s, 403)

    def test_correct_secret_but_disabled_200_ignored(self):
        CFG["auto_strm_enabled"] = False
        s, h, b = req("POST", "/api/cd2/webhook?key=" + self.SECRET, self._payload())
        self.assertEqual(s, 200)
        self.assertEqual(b.get("ignored"), "disabled")

    def test_correct_secret_enabled_200_ok(self):
        # 正确密钥 + 启用 → 200 ok(emby 离线时 fetch_libs 空 → queued 可能 0,但路由通)
        s, h, b = req("POST", "/api/cd2/webhook?key=" + self.SECRET, self._payload())
        self.assertEqual(s, 200)
        self.assertTrue(b.get("ok"))
        self.assertIn("queued", b)

    def test_secret_via_header(self):
        c = HTTPConnection("127.0.0.1", SERVER_PORT, timeout=10)
        c.request("POST", "/api/cd2/webhook",
                  json.dumps(self._payload()).encode(),
                  {"Content-Type": "application/json", "X-Webhook-Secret": self.SECRET})
        r = c.getresponse(); body = json.loads(r.read().decode()); c.close()
        self.assertEqual(r.status, 200)
        self.assertTrue(body.get("ok"))

    def test_webhook_never_requires_auth(self):
        # 即使 secret 为空(功能未配),也走密钥层 403,绝不是 401 未登录(证明在 _auth 之前分流)
        CFG["cd2_webhook_secret"] = ""
        s, h, b = req("POST", "/api/cd2/webhook?key=anything", self._payload())
        self.assertEqual(s, 403)
        self.assertNotEqual(b.get("err"), "未登录")


# ============================================================
# 13) /api/catalog/transfer:cid 校验 + 115/离线路由
# ============================================================
class CatalogTransferTests(unittest.TestCase):
    def setUp(self):
        self._saved_map = CFG.get("c115_cid_map")
        CFG["c115_cid_map"] = {"电影": "12345", "坏库": "0"}
        self.tok, self.csrf = login_get_token_csrf()

    def tearDown(self):
        if self._saved_map is None: CFG.pop("c115_cid_map", None)
        else: CFG["c115_cid_map"] = self._saved_map

    def _post(self, body):
        return req("POST", "/api/catalog/transfer", body,
                   cookie="emby_tok=" + self.tok, csrf=self.csrf)

    def test_rejects_invalid_explicit_cid_before_115_call(self):
        with patch.object(app, "c115_save_to_cid") as save, patch.object(app, "c115_offline_add") as off:
            s, h, b = self._post({"cid": "0", "link": "https://115cdn.com/s/swabc"})
        self.assertEqual(s, 400)
        self.assertIn("目标 cid 非法", b.get("err", ""))
        save.assert_not_called()
        off.assert_not_called()

    def test_rejects_invalid_mapped_cid_before_115_call(self):
        with patch.object(app, "c115_save_to_cid") as save, patch.object(app, "c115_offline_add") as off:
            s, h, b = self._post({"lib": "坏库", "link": "magnet:?xt=urn:btih:abc"})
        self.assertEqual(s, 400)
        self.assertIn("目标 cid 非法", b.get("err", ""))
        save.assert_not_called()
        off.assert_not_called()

    def test_115_share_routes_to_save_to_cid(self):
        with patch.object(app, "c115_save_to_cid", return_value={"ok": True, "count": 1}) as save, \
             patch.object(app, "c115_offline_add") as off:
            s, h, b = self._post({"lib": "电影", "link": "https://115cdn.com/s/swabc?password=1", "label": "片名"})
        self.assertEqual(s, 200)
        self.assertTrue(b.get("ok"))
        save.assert_called_once_with("https://115cdn.com/s/swabc?password=1", "", "12345", label="片名")
        off.assert_not_called()

    def test_magnet_routes_to_offline_add(self):
        with patch.object(app, "c115_save_to_cid") as save, \
             patch.object(app, "c115_offline_add", return_value={"ok": True, "msg": "queued"}) as off:
            s, h, b = self._post({"lib": "电影", "link": "magnet:?xt=urn:btih:abc", "label": "片名"})
        self.assertEqual(s, 200)
        self.assertTrue(b.get("ok"))
        off.assert_called_once_with("magnet:?xt=urn:btih:abc", "12345", label="片名")
        save.assert_not_called()


if __name__ == "__main__":
    unittest.main()
