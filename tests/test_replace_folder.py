"""replace_folder 的 Emby 通知回归(bug:旧路径发 Modified 清不掉 → Emby 孤儿重复剧集)。
两个改名方向都要:消失的路径发 Deleted,被占用/保留的路径发 Modified/Created。纯文件系统 + mock epost,不触网。"""
import os, sys, tempfile, unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import lib.business as biz


def _run_replace(win, lose, make_rename_strm=False):
    """搭 CD/STRM 临时目录跑 replace_folder,返回 epost 收到的 Updates 列表。"""
    fol = "电视剧追更"
    captured = {}
    with tempfile.TemporaryDirectory() as cd, tempfile.TemporaryDirectory() as strm:
        # 115(CD)两个 folder 都在
        os.makedirs(os.path.join(cd, fol, win))
        os.makedirs(os.path.join(cd, fol, lose))
        # win 的 strm 必在(replace 要么删要么改名它)
        win_strm = os.path.join(strm, fol, win)
        os.makedirs(win_strm)
        with open(os.path.join(win_strm, "S01E01.strm"), "w") as f:
            f.write("/media/%s/%s/S01E01.mkv" % (fol, win))
        L = {"电视剧追更": {"id": "1", "ctype": "tvshows", "folder": fol}}

        def fake_epost(path, body=None, **k):
            if path == "/Library/Media/Updated":
                captured["updates"] = body.get("Updates")
            return 204

        with patch.object(biz, "CD", cd), patch.object(biz, "STRM", strm), \
             patch.object(biz, "fetch_libs", return_value=L), \
             patch.object(biz, "epost", fake_epost), \
             patch.object(biz, "_undo_record", lambda *a, **k: None):
            r = biz.replace_folder("电视剧追更", win, lose)
    return r, captured.get("updates", []), fol


def _find(updates, path_suffix):
    for u in updates:
        if u.get("Path", "").endswith(path_suffix):
            return u.get("UpdateType")
    return None


class TestReplaceFolderNotify(unittest.TestCase):
    def test_keep_basename_drop_suffix_sends_deleted(self):
        """留原名、删 (1):被删的 (1) 路径必须发 Deleted(原 bug 发的是 Modified → 孤儿)。"""
        win = "某剧(2026)[tmdbid-9]"
        lose = "某剧(2026)[tmdbid-9](1)"
        r, updates, fol = _run_replace(win, lose)
        self.assertTrue(r.get("ok"))
        self.assertEqual(r.get("kept_as"), win)  # 没改名
        self.assertEqual(_find(updates, "/%s/%s" % (fol, lose)), "Deleted",
                         "被删的 (1) 路径必须 Deleted,否则 Emby 留孤儿重复剧集")
        self.assertEqual(_find(updates, "/%s/%s" % (fol, win)), "Created",
                         "保留的 win 路径发 Created 确保收录")

    def test_rename_suffix_to_basename_old_winpath_deleted(self):
        """删原名、(1) 改名回原名:win 旧 (1) 路径没了 → Deleted;lose 路径被占用 → Modified。"""
        win = "某剧(2026)[tmdbid-9](1)"
        lose = "某剧(2026)[tmdbid-9]"
        r, updates, fol = _run_replace(win, lose)
        self.assertTrue(r.get("ok"))
        self.assertEqual(r.get("kept_as"), lose)  # win 改名回 lose 名
        self.assertEqual(_find(updates, "/%s/%s" % (fol, win)), "Deleted",
                         "win 的旧 (1) 路径已不存在,必须 Deleted")
        self.assertEqual(_find(updates, "/%s/%s" % (fol, lose)), "Modified",
                         "lose 路径现由 win 内容占用 → Modified 就地更新")

    def test_no_stale_modified_on_missing_path(self):
        """任何方向都不该对一个已不存在的路径发 Modified(那正是原 bug)。"""
        for win, lose in [("某剧[tmdbid-9]", "某剧[tmdbid-9](1)"),
                          ("某剧[tmdbid-9](1)", "某剧[tmdbid-9]")]:
            r, updates, fol = _run_replace(win, lose)
            for u in updates:
                if u.get("UpdateType") == "Modified":
                    # Modified 只允许打在改名后仍存在的路径上(kept_as 那个)
                    self.assertTrue(u.get("Path", "").endswith(r.get("kept_as")),
                                    "Modified 不能打在已删除的路径 %s" % u.get("Path"))


if __name__ == "__main__":
    unittest.main()
