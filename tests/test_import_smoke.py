"""Smoke test:确认 app.py 能 import,关键纯函数都在,且没被改坏。
任何把 app.py 写出语法错 / NameError 的改动,这条都会 fail。"""
import os, sys, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


class TestImportSmoke(unittest.TestCase):

    def test_import_succeeds(self):
        import app  # noqa: F401
        self.assertTrue(True)

    def test_key_pure_functions_exist(self):
        import app
        for name in [
            "qscore",
            "_safe_under",
            "_hash_password",
            "_verify_password",
            "c115_parse_url",
            "c115_snap_full",
            "c115_list_dirs",
            "analyze_dups",
            "series_gaps",
        ]:
            self.assertTrue(hasattr(app, name), "缺函数 %s" % name)
            self.assertTrue(callable(getattr(app, name)), "%s 不可调用" % name)

    def test_module_constants_sane(self):
        import app
        # 关键常量没被误删
        self.assertTrue(isinstance(app.VE, tuple) and len(app.VE) > 0)
        self.assertIn(".mkv", app.VE)
        self.assertTrue(isinstance(app.CFG, dict))
        self.assertIn("emby_url", app.CFG)
        self.assertIn("api_key", app.CFG)


if __name__ == "__main__":
    unittest.main()
