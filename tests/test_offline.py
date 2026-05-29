"""115 离线下载 c115_offline_add 测试:验 sign→add 两步流程 + 参数形态 + 错误处理。全 mock req_fn,不触网。"""
import os, sys, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from lib import c115


class TestOfflineAdd(unittest.TestCase):
    def _req(self, space_resp, add_resp):
        calls = []
        def req(path, params=None, post=None, cookie=None, host=None):
            calls.append({"path": path, "params": params, "post": post, "host": host})
            if params and params.get("ac") == "space":
                return space_resp
            return add_resp
        return req, calls

    def test_add_success_flow(self):
        req, calls = self._req(
            {"state": True, "sign": "SIGN123", "time": 1780000000, "quota": 2997},
            {"state": True, "info_hash": "abc123"})
        r = c115.c115_offline_add(req, "magnet:?xt=urn:btih:abc", "789", label="电影")
        self.assertTrue(r["ok"]); self.assertEqual(r["info_hash"], "abc123"); self.assertEqual(r["quota"], 2997)
        # 第一步拿 sign 用主站域名
        self.assertEqual(calls[0]["host"], c115.C115_SITE)
        self.assertEqual(calls[0]["params"], {"ct": "offline", "ac": "space"})
        # 第二步 add_task_url 带 url/wp_path_id/sign/time
        add = calls[1]
        self.assertEqual(add["params"], {"ct": "lixian", "ac": "add_task_url"})
        self.assertEqual(add["post"]["url"], "magnet:?xt=urn:btih:abc")
        self.assertEqual(add["post"]["wp_path_id"], "789")
        self.assertEqual(add["post"]["sign"], "SIGN123")
        self.assertEqual(add["post"]["time"], 1780000000)
        self.assertEqual(add["host"], c115.C115_SITE)

    def test_sign_failure_aborts(self):
        # 拿 sign 失败(cookie 失效/无离线权限)→ 不该再发 add
        req, calls = self._req({"state": False, "error": "未登录"}, {"state": True})
        r = c115.c115_offline_add(req, "magnet:x", "1")
        self.assertFalse(r["ok"]); self.assertIn("sign", r["err"])
        self.assertEqual(len(calls), 1, "拿 sign 失败后不应再调 add")

    def test_add_rejected(self):
        req, _ = self._req({"state": True, "sign": "S", "time": 1},
                           {"state": False, "error_msg": "任务已存在", "errcode": 911})
        r = c115.c115_offline_add(req, "ed2k://x", "1")
        self.assertFalse(r["ok"]); self.assertIn("任务已存在", r["err"])

    def test_empty_url_or_cid(self):
        req, calls = self._req({"state": True, "sign": "S", "time": 1}, {"state": True})
        self.assertFalse(c115.c115_offline_add(req, "", "1")["ok"])
        self.assertFalse(c115.c115_offline_add(req, "magnet:x", "")["ok"])
        self.assertEqual(calls, [], "参数缺失应在发请求前就返回")


if __name__ == "__main__":
    unittest.main()
