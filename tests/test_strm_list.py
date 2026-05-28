"""list_strm() 测试:列指定 lib/folder 下所有 strm 文件,带 target 内容。
覆盖:happy path / 未知 lib(AppError 404) / path traversal(ValueError)。
"""
import os, sys, tempfile, unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import lib.business as biz
from lib.logger import AppError


class TestListStrm(unittest.TestCase):

    def setUp(self):
        # 临时 STRM 根,塞两个 strm
        self.tmp = tempfile.mkdtemp(prefix="strm_list_test_")
        self.lib_folder = "test_lib_folder"  # 仅文件夹名,不同于 lib 显示名
        base = os.path.join(self.tmp, self.lib_folder, "test_folder")
        os.makedirs(base, exist_ok=True)
        with open(os.path.join(base, "a.strm"), "w", encoding="utf-8") as f:
            f.write("/media/test_lib_folder/test_folder/a.mkv")
        with open(os.path.join(base, "b.strm"), "w", encoding="utf-8") as f:
            f.write("/media/test_lib_folder/test_folder/b.mkv")
        # 一个非 strm 干扰
        with open(os.path.join(base, "ignore.nfo"), "w") as f:
            f.write("<nfo/>")
        self._old_STRM = biz.STRM
        biz.STRM = self.tmp

    def tearDown(self):
        biz.STRM = self._old_STRM
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_lists_strm_files_with_target(self):
        # mock fetch_libs 返回 test_lib → folder
        with patch.object(biz, "fetch_libs",
                          return_value={"test_lib": {"id": "1", "ctype": "tvshows", "folder": self.lib_folder}}):
            r = biz.list_strm("test_lib", "test_folder")
        self.assertEqual(r["lib"], "test_lib")
        self.assertEqual(r["folder"], "test_folder")
        self.assertEqual(len(r["files"]), 2)
        names = sorted([f["rel"] for f in r["files"]])
        self.assertEqual(names, ["a.strm", "b.strm"])
        for f in r["files"]:
            self.assertTrue(f["target"].startswith("/media/"))

    def test_unknown_lib_raises_apperror_404(self):
        with patch.object(biz, "fetch_libs", return_value={}):
            with self.assertRaises(AppError) as ctx:
                biz.list_strm("unknown_lib", "test_folder")
        self.assertEqual(ctx.exception.status, 404)

    def test_path_traversal_rejected(self):
        with patch.object(biz, "fetch_libs",
                          return_value={"test_lib": {"id": "1", "ctype": "tvshows", "folder": self.lib_folder}}):
            with self.assertRaises(ValueError):
                biz.list_strm("test_lib", "../etc")

    def test_nonexistent_folder_returns_empty_files(self):
        with patch.object(biz, "fetch_libs",
                          return_value={"test_lib": {"id": "1", "ctype": "tvshows", "folder": self.lib_folder}}):
            r = biz.list_strm("test_lib", "no_such_folder")
        self.assertEqual(r["files"], [])


if __name__ == "__main__":
    unittest.main()
