"""autostrm 单元测试:路径反映射 / webhook payload 解析 / 防抖合并 / strm 写入幂等 /
全自动生成 / seen 状态 / 延迟匹配状态机。不打网络(fetch_libs/eget 全 mock)。
"""
import os, sys, json, tempfile, shutil, unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from lib.config import CFG
import lib.autostrm as a
import lib.business as biz

LIBS = {"电影": {"id": "119034", "ctype": "movies", "folder": "电影"},
        "动漫": {"id": "96247", "ctype": "tvshows", "folder": "动漫"}}


def _reset_autostrm_state():
    a._PENDING.clear()
    a._MATCH.clear()
    a._RECENT.clear()
    a._UNMATCHED.clear()
    a._UNMAPPED.clear()
    a._LIBS_CACHE["ts"] = 0.0
    a._LIBS_CACHE["map"] = {}
    a._SEEN = None


# ============================================================
# 路径反映射
# ============================================================
class ReverseMapTests(unittest.TestCase):
    def setUp(self):
        _reset_autostrm_state()
        self._old = CFG.get("cd2_mount_prefix")
        CFG["cd2_mount_prefix"] = "/CloudNAS/CloudDrive"

    def tearDown(self):
        if self._old is None:
            CFG.pop("cd2_mount_prefix", None)
        else:
            CFG["cd2_mount_prefix"] = self._old

    def test_maps_known_folder(self):
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            r = a._reverse_map("/CloudNAS/CloudDrive/电影/教父 (1972)/godfather.mkv")
        self.assertEqual(r, ("电影", "教父 (1972)"))

    def test_nested_path_top_is_first_segment(self):
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            r = a._reverse_map("/CloudNAS/CloudDrive/动漫/海贼王/Season01/ep1000.mkv")
        self.assertEqual(r, ("动漫", "海贼王"))

    def test_wrong_prefix_returns_none(self):
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            self.assertIsNone(a._reverse_map("/115/电影/x/a.mkv"))

    def test_unknown_folder_returns_none(self):
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            self.assertIsNone(a._reverse_map("/CloudNAS/CloudDrive/未知库/x/a.mkv"))

    def test_no_inner_path_returns_none(self):
        # 库根本身(只到 folder,没有 top)→ None
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            self.assertIsNone(a._reverse_map("/CloudNAS/CloudDrive/电影"))
            self.assertIsNone(a._reverse_map("/CloudNAS/CloudDrive/电影/"))

    def test_custom_prefix(self):
        CFG["cd2_mount_prefix"] = "/115/emby"
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            r = a._reverse_map("/115/emby/电影/片 (2024)/a.mkv")
        self.assertEqual(r, ("电影", "片 (2024)"))


# ============================================================
# webhook payload 解析 + 入队 + 过滤
# ============================================================
class HandleEventTests(unittest.TestCase):
    def setUp(self):
        _reset_autostrm_state()
        CFG["cd2_mount_prefix"] = "/CloudNAS/CloudDrive"

    def _ev(self, **kw):
        e = {"action": "create", "is_dir": False, "source_file": "", "destination_file": ""}
        e.update(kw); return e

    def test_video_event_queued(self):
        payload = {"data": [self._ev(source_file="/CloudNAS/CloudDrive/电影/片 (2024)/a.mkv")]}
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            n = a.handle_cd2_event(payload)
        self.assertEqual(n, 1)
        self.assertIn(("电影", "片 (2024)"), a._PENDING)

    def test_non_video_ignored(self):
        payload = {"data": [self._ev(source_file="/CloudNAS/CloudDrive/电影/片 (2024)/poster.jpg"),
                            self._ev(source_file="/CloudNAS/CloudDrive/电影/片 (2024)/x.nfo")]}
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            n = a.handle_cd2_event(payload)
        self.assertEqual(n, 0)
        self.assertEqual(len(a._PENDING), 0)

    def test_dir_event_ignored(self):
        payload = {"data": [self._ev(is_dir=True, source_file="/CloudNAS/CloudDrive/电影/新文件夹")]}
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            self.assertEqual(a.handle_cd2_event(payload), 0)

    def test_is_dir_string_true_ignored(self):
        # CD2 可能把 is_dir 发成字符串 "True"
        payload = {"data": [self._ev(is_dir="True", source_file="/CloudNAS/CloudDrive/电影/x/a.mkv")]}
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            self.assertEqual(a.handle_cd2_event(payload), 0)

    def test_delete_action_ignored(self):
        payload = {"data": [self._ev(action="delete", source_file="/CloudNAS/CloudDrive/电影/片/a.mkv")]}
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            self.assertEqual(a.handle_cd2_event(payload), 0)

    def test_move_uses_destination(self):
        payload = {"data": [self._ev(action="move",
                                     source_file="/somewhere/old.mkv",
                                     destination_file="/CloudNAS/CloudDrive/动漫/新番/ep1.mkv")]}
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            n = a.handle_cd2_event(payload)
        self.assertEqual(n, 1)
        self.assertIn(("动漫", "新番"), a._PENDING)

    def test_coalesce_same_top(self):
        # 同一 (lib, top) 的多文件事件 → 防抖 dict 只一个 key(一个 burst 一次)
        evs = [self._ev(source_file="/CloudNAS/CloudDrive/动漫/海贼王/Season01/ep%d.mkv" % i)
               for i in range(30)]
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            n = a.handle_cd2_event({"data": evs})
        self.assertEqual(n, 30)               # 30 个事件都被接受
        self.assertEqual(len(a._PENDING), 1)  # 但只合并成 1 个待处理 key
        self.assertIn(("动漫", "海贼王"), a._PENDING)

    def test_unmapped_recorded(self):
        payload = {"data": [self._ev(source_file="/wrong/prefix/电影/x/a.mkv")]}
        with patch("lib.emby.fetch_libs", return_value=LIBS):
            a.handle_cd2_event(payload)
        self.assertEqual(len(a._UNMAPPED), 1)

    def test_bad_payload_no_crash(self):
        self.assertEqual(a.handle_cd2_event(None), 0)
        self.assertEqual(a.handle_cd2_event({}), 0)
        self.assertEqual(a.handle_cd2_event({"data": "notalist"}), 0)


# ============================================================
# strm 写入(_write_strm 幂等)+ 全自动生成
# ============================================================
class WriteStrmTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="autostrm_test_")
        self.strm_base = os.path.join(self.tmp, "电影")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_writes_media_local_path(self):
        p = biz._write_strm(self.strm_base, "/media/电影", "片 (2024)", "a.mkv")
        self.assertIsNotNone(p)
        with open(p, encoding="utf-8") as f:
            self.assertEqual(f.read(), "/media/电影/片 (2024)/a.mkv")

    def test_idempotent_no_overwrite(self):
        biz._write_strm(self.strm_base, "/media/电影", "片", "a.mkv")
        again = biz._write_strm(self.strm_base, "/media/电影", "片", "a.mkv")
        self.assertIsNone(again)   # 已存在 → None,不覆盖


class GenStrmFullautoTests(unittest.TestCase):
    """gen_strm_for_lib_path:全自动 vs 谨慎,对无 tmdbid 文件夹的行为。"""
    def setUp(self):
        _reset_autostrm_state()
        self.tmp = tempfile.mkdtemp(prefix="gen_test_")
        self.cd = os.path.join(self.tmp, "cd"); self.strm = os.path.join(self.tmp, "strm")
        # 115 侧:电影/无番号片/a.mkv
        os.makedirs(os.path.join(self.cd, "电影", "无番号片 (2024)"))
        with open(os.path.join(self.cd, "电影", "无番号片 (2024)", "a.mkv"), "w") as f:
            f.write("x")
        self._oldCD, self._oldSTRM = biz.CD, biz.STRM
        biz.CD = self.cd; biz.STRM = self.strm

    def tearDown(self):
        biz.CD, biz.STRM = self._oldCD, self._oldSTRM
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_fullauto_generates_and_flags_match(self):
        with patch.object(biz, "fetch_libs", return_value=LIBS), \
             patch.object(biz, "_mount_alive", return_value=True), \
             patch.object(biz, "epost", return_value=200):
            r = biz.gen_strm_for_lib_path("电影", "无番号片 (2024)", fullauto=True)
        self.assertEqual(r["new_count"], 1)
        self.assertTrue(r["needs_match"])     # 无 tmdbid → 需延迟匹配
        # strm 真写了,内容是 /media 本地路径
        sp = os.path.join(self.strm, "电影", "无番号片 (2024)", "a.strm")
        self.assertTrue(os.path.exists(sp))
        with open(sp, encoding="utf-8") as f:
            self.assertEqual(f.read(), "/media/电影/无番号片 (2024)/a.mkv")

    def test_conservative_skips_no_tmdb(self):
        with patch.object(biz, "fetch_libs", return_value=LIBS), \
             patch.object(biz, "_mount_alive", return_value=True), \
             patch.object(biz, "epost", return_value=200):
            r = biz.gen_strm_for_lib_path("电影", "无番号片 (2024)", fullauto=False)
        self.assertEqual(r["new_count"], 0)   # 非全自动 + 无 tmdbid → 不生成
        self.assertIn("attention", r)

    def test_mount_dead_skips(self):
        with patch.object(biz, "fetch_libs", return_value=LIBS), \
             patch.object(biz, "_mount_alive", return_value=False):
            r = biz.gen_strm_for_lib_path("电影", "无番号片 (2024)", fullauto=True)
        self.assertEqual(r.get("skipped"), "mount_dead")
        self.assertEqual(r["new_count"], 0)

    def test_path_traversal_rejected(self):
        with patch.object(biz, "fetch_libs", return_value=LIBS):
            r = biz.gen_strm_for_lib_path("电影", "../../../etc", fullauto=True)
        self.assertIn("err", r)
        self.assertEqual(r["new_count"], 0)


# ============================================================
# seen 状态(sidecar）
# ============================================================
class SeenStateTests(unittest.TestCase):
    def setUp(self):
        _reset_autostrm_state()
        self.tmp = tempfile.mkdtemp(prefix="seen_test_")
        self._old = a.STRM
        a.STRM = self.tmp

    def tearDown(self):
        a.STRM = self._old
        a._SEEN = None
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_new_top_is_new(self):
        self.assertTrue(a.seen_is_new("电影", "片", 100.0))

    def test_marked_then_not_new_unless_mtime_advances(self):
        a.seen_mark("电影", "片", 100.0)
        self.assertFalse(a.seen_is_new("电影", "片", 100.0))
        self.assertFalse(a.seen_is_new("电影", "片", 99.0))
        self.assertTrue(a.seen_is_new("电影", "片", 101.0))  # mtime 前进 → 重新算新

    def test_persist_roundtrip(self):
        a.seen_mark("动漫", "海贼王", 12345.0)
        a.seen_save()
        a._SEEN = None  # 强制重读
        self.assertFalse(a.seen_is_new("动漫", "海贼王", 12345.0))
        self.assertTrue(os.path.exists(os.path.join(self.tmp, "autostrm_seen.json")))


# ============================================================
# status() 形状
# ============================================================
class StatusTests(unittest.TestCase):
    def setUp(self):
        _reset_autostrm_state()

    def test_status_shape(self):
        s = a.status()
        for k in ("enabled", "fullauto", "prefix", "secret_set", "pending_gen",
                  "pending_match", "dropped", "recent", "unmatched", "unmapped"):
            self.assertIn(k, s)
        self.assertIsInstance(s["recent"], list)


# ============================================================
# 审查修复回归:匹配去重 / Refresh 批处理(do_refresh)
# ============================================================
class ReviewFixTests(unittest.TestCase):
    def setUp(self):
        _reset_autostrm_state()

    def test_enqueue_match_dedupes(self):
        a.enqueue_match("电影", "片 (2024)", "119034", "电影")
        a.enqueue_match("电影", "片 (2024)", "119034", "电影")  # 同 (lib,top) 再入
        self.assertEqual(len(a._MATCH), 1)                      # 不重复
        a.enqueue_match("电影", "别的片", "119034", "电影")
        self.assertEqual(len(a._MATCH), 2)

    def test_enqueue_match_skips_without_libid(self):
        a.enqueue_match("电影", "片", None, "电影")
        a.enqueue_match("电影", "片", "119034", None)
        self.assertEqual(len(a._MATCH), 0)


class DoRefreshTests(unittest.TestCase):
    """gen_strm_for_lib_path do_refresh=False 仍写 strm、返回 lib_id,但不发 Emby Refresh(批处理统一刷)。"""
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="dorefresh_test_")
        self.cd = os.path.join(self.tmp, "cd"); self.strm = os.path.join(self.tmp, "strm")
        os.makedirs(os.path.join(self.cd, "电影", "tmdbid-123 片"))
        with open(os.path.join(self.cd, "电影", "tmdbid-123 片", "a.mkv"), "w") as f:
            f.write("x")
        self._oldCD, self._oldSTRM = biz.CD, biz.STRM
        biz.CD = self.cd; biz.STRM = self.strm

    def tearDown(self):
        biz.CD, biz.STRM = self._oldCD, self._oldSTRM
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_no_refresh_when_do_refresh_false(self):
        with patch.object(biz, "fetch_libs", return_value=LIBS), \
             patch.object(biz, "_mount_alive", return_value=True), \
             patch.object(biz, "epost", return_value=200) as ep:
            r = biz.gen_strm_for_lib_path("电影", "tmdbid-123 片", do_refresh=False)
        self.assertEqual(r["new_count"], 1)
        ep.assert_not_called()                # 没刷新
        self.assertEqual(r.get("lib_id"), "119034")  # 但回 lib_id 给批处理统一刷
        self.assertTrue(os.path.exists(os.path.join(self.strm, "电影", "tmdbid-123 片", "a.strm")))

    def test_refresh_when_do_refresh_true(self):
        with patch.object(biz, "fetch_libs", return_value=LIBS), \
             patch.object(biz, "_mount_alive", return_value=True), \
             patch.object(biz, "epost", return_value=200) as ep:
            biz.gen_strm_for_lib_path("电影", "tmdbid-123 片", do_refresh=True)
        ep.assert_called_once()


if __name__ == "__main__":
    unittest.main()
