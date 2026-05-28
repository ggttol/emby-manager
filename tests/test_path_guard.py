"""_safe_under(base, name) 测试:路径合法性 / path traversal 防护。
合法的应返回 realpath 且仍在 base 内;非法的必须 raise ValueError。"""
import os, sys, tempfile, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import app


class TestSafeUnder(unittest.TestCase):

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = self.tmp.name
        # 准备一个子目录,便于测试嵌套合法路径
        os.makedirs(os.path.join(self.base, "sub"), exist_ok=True)

    def tearDown(self):
        self.tmp.cleanup()

    # -------- 合法 --------
    def test_simple_filename(self):
        out = app._safe_under(self.base, "a.txt")
        self.assertTrue(out.endswith("/a.txt") or out.endswith("\\a.txt"))
        # realpath 应在 base 树内
        self.assertTrue(out.startswith(os.path.realpath(self.base) + os.sep))

    def test_nested_subdir(self):
        out = app._safe_under(self.base, "sub/file.mkv")
        self.assertTrue(out.startswith(os.path.realpath(self.base) + os.sep))

    def test_chinese_and_space_in_name(self):
        out = app._safe_under(self.base, "中文 带空格.mkv")
        self.assertTrue(out.startswith(os.path.realpath(self.base) + os.sep))

    def test_dotted_filename_is_ok(self):
        # "file.tar.gz" 不是 ".." — 必须允许
        out = app._safe_under(self.base, "archive.tar.gz")
        self.assertTrue(out.startswith(os.path.realpath(self.base) + os.sep))

    # -------- 非法 --------
    def test_reject_dot_dot(self):
        with self.assertRaises(ValueError):
            app._safe_under(self.base, "..")

    def test_reject_single_dot(self):
        with self.assertRaises(ValueError):
            app._safe_under(self.base, ".")

    def test_reject_empty_string(self):
        with self.assertRaises(ValueError):
            app._safe_under(self.base, "")

    def test_reject_null_byte(self):
        with self.assertRaises(ValueError):
            app._safe_under(self.base, "evil\x00.txt")

    def test_reject_parent_relative(self):
        with self.assertRaises(ValueError):
            app._safe_under(self.base, "../etc")

    def test_reject_absolute_path(self):
        with self.assertRaises(ValueError):
            app._safe_under(self.base, "/etc/passwd")

    def test_reject_nested_traversal(self):
        with self.assertRaises(ValueError):
            app._safe_under(self.base, "sub/../../etc")

    def test_reject_windows_backslash_traversal(self):
        # 反斜杠先 normalize 成 / 再判断 .. 段
        with self.assertRaises(ValueError):
            app._safe_under(self.base, "..\\foo")

    def test_reject_none(self):
        # None 也算"无效路径" — 函数应在 falsy 分支 raise
        with self.assertRaises((ValueError, TypeError, AttributeError)):
            app._safe_under(self.base, None)

    def test_reject_only_dots_segment(self):
        # "sub/.." 也是逃出 base 的写法
        with self.assertRaises(ValueError):
            app._safe_under(self.base, "sub/..")


if __name__ == "__main__":
    unittest.main()
