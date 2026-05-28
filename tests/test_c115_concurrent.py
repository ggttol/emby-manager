"""c115_test(cookie_override) 并发安全测试 — P0-1 regression。

P0-1:`c115_test(req_fn, cookie_override)` swap `CFG['c115_cookie']` 没加锁 →
并发污染真 cookie。预期修复方向:`c115_test` 内部用 `CFG_LOCK` 包 swap+restore,
让真 cookie 不被并发场景下另一个 thread 的 override 覆盖到永久状态。
"""
import os, sys, threading, time, unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from lib.config import CFG
from lib.c115 import c115_test


def reset_cfg(cookie="UID=REAL; SEID=real_seid"):
    CFG.clear()
    CFG["c115_cookie"] = cookie


class C115TestRaceTests(unittest.TestCase):

    def test_cookie_override_doesnt_leak_after_concurrent(self):
        """P0-1 regression: 并发 c115_test(cookie_override=X) 完成后,CFG[c115_cookie] 必须 == 原值。

        race scenario(无锁时复现):
          T1 进入 → 读 old="REAL" → 写 CFG="fake1" → 进 req_fn(等)
          T2 进入 → 读 old="fake1"(被污染的当前值) → 写 CFG="fake2" → 进 req_fn(等)
          T1 退出 → CFG=old="REAL"
          T2 退出 → CFG=old="fake1" ← 永久泄漏!
        加锁版本里整个 swap+req+restore 是原子的,不会出现 T2 读到 T1 的污染值。
        """
        reset_cfg(cookie="REAL_COOKIE_DO_NOT_OVERWRITE")

        # 用 event 强制 T2 必须在 T1 已经 swap 之后才开始
        t1_swapped = threading.Event()
        t1_can_restore = threading.Event()

        def t1_req(path, params=None, post=None):
            t1_swapped.set()           # 通知 T2 可以进
            t1_can_restore.wait(2.0)   # 等 T2 也 swap 完
            return {"state": True, "data": {"space_info": {"all_total": {"size_format": "1G"}}}}

        def t2_req(path, params=None, post=None):
            # T2 进到这里时已经 swap 过了,放 T1 走
            t1_can_restore.set()
            time.sleep(0.05)           # 给 T1 时间完成 restore
            return {"state": True, "data": {"space_info": {"all_total": {"size_format": "1G"}}}}

        def worker_t1():
            c115_test(t1_req, cookie_override="fake_T1")

        def worker_t2():
            t1_swapped.wait(2.0)       # 必须等 T1 先 swap
            c115_test(t2_req, cookie_override="fake_T2")

        t1 = threading.Thread(target=worker_t1)
        t2 = threading.Thread(target=worker_t2)
        t1.start(); t2.start()
        t1.join(timeout=5); t2.join(timeout=5)

        # 关键断言:race 后真 cookie 必须保留(无锁版本会留下 "fake_T1")
        self.assertEqual(CFG["c115_cookie"], "REAL_COOKIE_DO_NOT_OVERWRITE",
                         "并发 c115_test 污染了真 cookie!CFG[c115_cookie]=%r" % CFG.get("c115_cookie"))

    def test_cookie_override_uses_provided_not_cfg(self):
        """单线程基本路径:cookie_override 提供时,req_fn 收到的应该是 override 不是 CFG"""
        reset_cfg(cookie="REAL")
        seen = []

        def req(path, **kw):
            seen.append(CFG.get("c115_cookie"))
            return {"state": True, "data": {"space_info": {"all_total": {"size_format": "1G"}}}}

        c115_test(req, cookie_override="OVERRIDE_VALUE")
        self.assertIn("OVERRIDE_VALUE", seen, "req_fn 应该看到 override 值")
        # 调完恢复
        self.assertEqual(CFG["c115_cookie"], "REAL")

    def test_no_override_uses_cfg(self):
        """cookie_override=None 时不 swap,直接用 CFG 值"""
        reset_cfg(cookie="REAL_X")
        seen = []

        def req(path, **kw):
            seen.append(CFG.get("c115_cookie"))
            return {"state": True, "data": {"space_info": {"all_total": {"size_format": "1G"}}}}

        c115_test(req, cookie_override=None)
        self.assertEqual(seen, ["REAL_X"])
        self.assertEqual(CFG["c115_cookie"], "REAL_X")

    def test_empty_string_override_doesnt_corrupt(self):
        """边界:cookie_override='' 不应导致永久污染(即使 truthy 判断不同也要恢复)"""
        reset_cfg(cookie="REAL_Y")

        def req(path, **kw):
            return {"state": False, "error": "empty cookie"}

        try:
            c115_test(req, cookie_override="")
        except Exception:
            pass
        self.assertEqual(CFG["c115_cookie"], "REAL_Y")

    def test_exception_in_req_still_restores(self):
        """req_fn 抛异常时也必须 restore"""
        reset_cfg(cookie="REAL_Z")

        def boom(path, **kw):
            raise RuntimeError("simulated network err")

        try:
            c115_test(boom, cookie_override="evil")
        except Exception:
            pass
        self.assertEqual(CFG["c115_cookie"], "REAL_Z")


if __name__ == "__main__":
    unittest.main()
