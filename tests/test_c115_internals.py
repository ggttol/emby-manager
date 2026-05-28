"""c115_list_dirs / c115_snap_full 解析逻辑测试。

这两个函数本身要调 _c115_req(对外网请求),但**响应解析逻辑是纯函数** ——
mock 掉 _c115_req 返回固定 dict,只验证我们正确解析了 115 的 JSON 形状。

不 mock urlopen(全部在 _c115_req 之上),不真的发任何请求。"""
import os, sys, unittest
from unittest.mock import patch
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import app


class TestC115ListDirs(unittest.TestCase):

    def test_only_folders_returned(self):
        # 文件夹有 cid+pid 没 fid;文件有 fid → 应被跳过
        mock_resp = {
            "state": True,
            "data": [
                {"cid": "111", "pid": "0", "n": "电影"},
                {"cid": "222", "pid": "0", "n": "电视剧"},
                {"fid": "999", "cid": "0", "n": "movie.mkv"},  # 文件 → skip
            ],
        }
        with patch.object(app, "_c115_req", return_value=mock_resp):
            r = app.c115_list_dirs("0")
        self.assertTrue(r["ok"])
        names = [it["name"] for it in r["items"]]
        self.assertIn("电影", names)
        self.assertIn("电视剧", names)
        self.assertNotIn("movie.mkv", names)

    def test_supports_name_alias(self):
        # 115 偶尔返回 name 而非 n
        mock_resp = {
            "state": True,
            "data": [{"cid": "1", "name": "alt_key_dir"}],
        }
        with patch.object(app, "_c115_req", return_value=mock_resp):
            r = app.c115_list_dirs("0")
        self.assertEqual(r["items"][0]["name"], "alt_key_dir")

    def test_skips_entries_without_cid(self):
        mock_resp = {
            "state": True,
            "data": [
                {"cid": "1", "n": "valid"},
                {"n": "no_cid_at_all"},  # 既无 fid 也无 cid → 跳
            ],
        }
        with patch.object(app, "_c115_req", return_value=mock_resp):
            r = app.c115_list_dirs("0")
        names = [it["name"] for it in r["items"]]
        self.assertEqual(names, ["valid"])

    def test_cid_is_string(self):
        # 后端可能返回 int — 我们必须 str() 化(后面拼 URL 用)
        mock_resp = {"state": True, "data": [{"cid": 12345, "n": "x"}]}
        with patch.object(app, "_c115_req", return_value=mock_resp):
            r = app.c115_list_dirs("0")
        self.assertEqual(r["items"][0]["cid"], "12345")
        self.assertIsInstance(r["items"][0]["cid"], str)

    def test_upstream_failure_returns_ok_false(self):
        mock_resp = {"state": False, "error": "cookie 失效"}
        with patch.object(app, "_c115_req", return_value=mock_resp):
            r = app.c115_list_dirs("0")
        self.assertFalse(r["ok"])
        self.assertIn("cookie", r["err"])
        self.assertEqual(r["items"], [])

    def test_empty_data_is_ok_with_empty_list(self):
        with patch.object(app, "_c115_req", return_value={"state": True, "data": []}):
            r = app.c115_list_dirs("0")
        self.assertTrue(r["ok"])
        self.assertEqual(r["items"], [])


class TestC115SnapFull(unittest.TestCase):

    def test_parses_file_and_folder(self):
        mock_resp = {
            "state": True,
            "data": {
                "shareinfo": {"share_title": "某分享", "file_size": 1024},
                "list": [
                    {"fid": "F1", "n": "movie.mkv", "s": 500},  # 文件 (有 fid)
                    {"cid": "C2", "n": "folder1"},                # 文件夹 (无 fid)
                ],
            },
        }
        with patch.object(app, "_c115_req", return_value=mock_resp):
            r = app.c115_snap_full("https://115.com/s/swABC?password=PP", None)
        self.assertTrue(r["ok"])
        self.assertEqual(r["share"], "swABC")
        self.assertEqual(r["rc"], "PP")
        self.assertEqual(r["share_title"], "某分享")
        self.assertEqual(r["file_size"], 1024)
        files = {f["name"]: f for f in r["files"]}
        self.assertEqual(files["movie.mkv"]["is_dir"], 0)
        self.assertEqual(files["movie.mkv"]["id"], "F1")
        self.assertEqual(files["movie.mkv"]["size"], 500)
        self.assertEqual(files["folder1"]["is_dir"], 1)
        self.assertEqual(files["folder1"]["id"], "C2")

    def test_url_parse_failure_returns_err_without_calling_api(self):
        # URL 解析不到 share_code → 不应该调 _c115_req
        called = {"n": 0}
        def fake(*a, **kw):
            called["n"] += 1
            return {"state": True, "data": {}}
        with patch.object(app, "_c115_req", side_effect=fake):
            r = app.c115_snap_full("!!!@@@", None)
        self.assertFalse(r["ok"])
        self.assertIn("share_code", r["err"])
        self.assertEqual(called["n"], 0)

    def test_upstream_failure_propagates(self):
        mock_resp = {"state": False, "error": "提取码错误"}
        with patch.object(app, "_c115_req", return_value=mock_resp):
            r = app.c115_snap_full("https://115.com/s/swABC?password=BAD", None)
        self.assertFalse(r["ok"])
        self.assertEqual(r["err"], "提取码错误")
        # 即便失败,share/rc 也要回传(供前端展示)
        self.assertEqual(r["share"], "swABC")
        self.assertEqual(r["rc"], "BAD")

    def test_explicit_pwd_arg_overrides_url(self):
        mock_resp = {
            "state": True,
            "data": {"shareinfo": {}, "list": [{"fid": "F1", "n": "x.mp4"}]},
        }
        captured = {}
        def fake(path, params=None, post=None):
            captured["params"] = params
            return mock_resp
        with patch.object(app, "_c115_req", side_effect=fake):
            r = app.c115_snap_full("https://115.com/s/swABC?password=URL", "OVERRIDE")
        self.assertTrue(r["ok"])
        self.assertEqual(r["rc"], "OVERRIDE")
        # 调 snap 时 receive_code 也要传 OVERRIDE
        self.assertEqual(captured["params"]["receive_code"], "OVERRIDE")

    def test_alternate_field_names_accepted(self):
        # 115 偶尔返回 file_id 而非 fid,name 而非 n,size 而非 s
        mock_resp = {
            "state": True,
            "data": {
                "shareinfo": {"file_name": "alt_title"},
                "list": [{"file_id": "F1", "name": "video.mp4", "size": 200}],
            },
        }
        with patch.object(app, "_c115_req", return_value=mock_resp):
            r = app.c115_snap_full("https://115.com/s/swABC", None)
        self.assertTrue(r["ok"])
        self.assertEqual(r["share_title"], "alt_title")
        self.assertEqual(r["files"][0]["name"], "video.mp4")
        self.assertEqual(r["files"][0]["size"], 200)


if __name__ == "__main__":
    unittest.main()
