"""c115_test(cookie_override) 隔离测试 — P0-1 regression(新契约)。

旧实现 swap 全局 CFG['c115_cookie'] 来传候选 cookie,并发会污染真 cookie 且会让并发的
批量 115 请求读到候选 cookie。新实现把候选 cookie 作为 `cookie=` 参数透传 req_fn,
**完全不碰全局 CFG**。本文件验证这个新契约。
"""
import os, sys, threading, unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from lib.config import CFG
from lib.c115 import c115_test


def reset_cfg(cookie="UID=REAL; SEID=real_seid"):
    CFG.clear()
    CFG["c115_cookie"] = cookie


_OK = {"state": True, "data": {"space_info": {"all_total": {"size_format": "1G"}}}}


class C115TestIsolationTests(unittest.TestCase):

    def test_override_passed_as_cookie_kwarg_not_cfg(self):
        """cookie_override 提供时:req_fn 收到 cookie= 参数,且全局 CFG 不被改。"""
        reset_cfg(cookie="REAL")
        seen = {}

        def req(path, params=None, post=None, cookie=None):
            seen["cookie"] = cookie
            seen["cfg_during"] = CFG.get("c115_cookie")
            return _OK

        c115_test(req, cookie_override="OVERRIDE_VALUE")
        self.assertEqual(seen["cookie"], "OVERRIDE_VALUE", "req_fn 应通过 cookie= 收到 override")
        self.assertEqual(seen["cfg_during"], "REAL", "全局 CFG 在调用期间不应被改")
        self.assertEqual(CFG["c115_cookie"], "REAL", "调用后 CFG 仍是真 cookie")

    def test_no_override_uses_cfg(self):
        """cookie_override=None:不传 cookie kwarg,req_fn 回落全局 CFG。"""
        reset_cfg(cookie="REAL_X")
        seen = {}

        def req(path, params=None, post=None, cookie=None):
            seen["cookie"] = cookie
            return _OK

        c115_test(req, cookie_override=None)
        self.assertIsNone(seen["cookie"], "无 override 时不应传 cookie kwarg(None=回落 CFG)")
        self.assertEqual(CFG["c115_cookie"], "REAL_X")

    def test_concurrent_overrides_never_touch_cfg(self):
        """并发跑多个 cookie_override 测试,全局 CFG 始终保持真 cookie(新实现根本不写 CFG)。"""
        reset_cfg(cookie="REAL_COOKIE_DO_NOT_OVERWRITE")

        def req(path, params=None, post=None, cookie=None):
            return _OK

        errs = []

        def worker(val):
            try:
                for _ in range(20):
                    c115_test(req, cookie_override="fake_" + val)
            except Exception as e:
                errs.append(e)

        ts = [threading.Thread(target=worker, args=(str(i),)) for i in range(4)]
        for t in ts: t.start()
        for t in ts: t.join(timeout=5)
        self.assertEqual(errs, [])
        self.assertEqual(CFG["c115_cookie"], "REAL_COOKIE_DO_NOT_OVERWRITE")

    def test_exception_in_req_doesnt_touch_cfg(self):
        reset_cfg(cookie="REAL_Z")

        def boom(path, params=None, post=None, cookie=None):
            raise RuntimeError("simulated network err")

        try:
            c115_test(boom, cookie_override="evil")
        except Exception:
            pass
        self.assertEqual(CFG["c115_cookie"], "REAL_Z")


if __name__ == "__main__":
    unittest.main()
