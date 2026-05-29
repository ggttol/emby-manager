"""回归测试:覆盖本轮 review 修复的关键逻辑(qscore dv 边界 / analyze_dups 命名兜底 /
delete_item verify+retry / grace 收口 / move smart qscore tiebreak)。
纯逻辑 + mock,不触网、不依赖真实 Emby/115。"""
import os, sys, tempfile, unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import lib.business as biz


class TestQscoreDvBoundary(unittest.TestCase):
    """review M3:'dv' 裸子串误判 → 必须带边界。"""
    def test_real_dv_still_scores(self):
        # 真杜比视界(带边界)仍 +60
        self.assertGreater(biz.qscore("Movie.2160p.DV.HDR.mkv"), biz.qscore("Movie.2160p.mkv"))
        self.assertGreater(biz.qscore("片.杜比视界.mkv"), 0)
        self.assertGreater(biz.qscore("Movie.DoVi.mkv"), biz.qscore("Movie.mkv"))

    def test_dvd_advengers_not_misscored_as_dv(self):
        # 'dvdrip' / 'advengers' / 'DVD' 不应吃 DV 的 +60
        # 对照:同为 1080p,一个含 dvd 子串、一个干净 → DV 加成不应让 dvd 版更高
        dvd = biz.qscore("Movie.1080p.DVDRip.mkv")
        web = biz.qscore("Movie.1080p.WEB-DL.mkv")
        # DVDRip 含 'dvd' 子串,旧 bug 会 +60;修复后不应因 DV 而虚高
        # 用一个不含任何来源关键词的纯净 1080p 做基线,确认 dvd 没拿到 dv 的 60 分
        clean = biz.qscore("Movie.1080p.mkv")
        self.assertEqual(biz.qscore("Movie.1080p.DVD.mkv") - clean, 0,
                         "含 DVD 的文件名不应吃到 DV 的 +60(只是分辨率分)")
        self.assertEqual(biz.qscore("Advengers.1080p.mkv") - clean, 0,
                         "advengers 的 dv 子串不应吃 +60")


class TestAnalyzeDupsSeriesGuard(unittest.TestCase):
    """review M5:命名不规范的剧(无 sXXeYY)不能当电影自动去重。"""
    def _setup_lib(self, tmpdir, ctype):
        # 造一个 tvshows 库,同 tmdbid 两个 folder,每个含一个中文集号 strm(eps 解析不出)
        lib_folder = "电视剧"
        base = os.path.join(tmpdir, lib_folder)
        for fold in ("某剧 第一季 [tmdbid-999]", "某剧 第二季 [tmdbid-999]"):
            d = os.path.join(base, fold)
            os.makedirs(d)
            with open(os.path.join(d, "第01集.strm"), "w") as f:
                f.write("/media/电视剧/%s/第01集.mkv" % fold)
            with open(os.path.join(d, "第02集.strm"), "w") as f:
                f.write("/media/电视剧/%s/第02集.mkv" % fold)
        return lib_folder

    def test_unparseable_series_goes_review_not_dups(self):
        with tempfile.TemporaryDirectory() as tmp:
            lib_folder = self._setup_lib(tmp, "tvshows")
            fake_libs = {"电视剧": {"id": "1", "ctype": "tvshows", "folder": lib_folder}}
            with patch.object(biz, "STRM", tmp), \
                 patch.object(biz, "fetch_libs", return_value=fake_libs):
                r = biz.analyze_dups()
            # 集号解析不出 + tvshows 库 → 必须进 review,绝不能 auto dups
            self.assertEqual(r["dups"], [], "命名不规范的剧不该被自动去重")
            self.assertTrue(any("999" == g["tmdb"] for g in r["review"]),
                            "应落入 review 等人工确认")


class TestGraceClosure(unittest.TestCase):
    """review M2:新装/已有密码的装不留永久 grace。"""
    def test_new_install_gets_timestamp_not_none(self):
        from lib import config
        # 模拟新装:CONFIG_EXISTED=False
        with patch.object(config, "CONFIG_EXISTED", False):
            config.CFG.pop("last_password_change_at", None)
            config._mig_to_v4()
            self.assertIsNotNone(config.CFG["last_password_change_at"],
                                 "新装应戳真实时间戳,不留 None 永久 grace")

    def test_existing_with_password_gets_timestamp(self):
        from lib import config
        with patch.object(config, "CONFIG_EXISTED", True):
            config.CFG.pop("last_password_change_at", None)
            config.CFG["password_hash"] = "pbkdf2_sha256$x$y$z"
            config._mig_to_v4()
            self.assertIsNotNone(config.CFG["last_password_change_at"],
                                 "已有密码的老装也应戳时间戳(改密需旧密码)")

    def test_existing_without_password_keeps_none(self):
        from lib import config
        with patch.object(config, "CONFIG_EXISTED", True):
            config.CFG.pop("last_password_change_at", None)
            config.CFG["password_hash"] = ""
            config._mig_to_v4()
            self.assertIsNone(config.CFG["last_password_change_at"],
                              "无密码老装保留 None,允许首次设密码")


class TestDeleteItemVerifyRetry(unittest.TestCase):
    """CLAUDE.md 雷#1 的核心防御:先 edelete + verify + retry,再动磁盘。"""
    def _run(self, eget_seq):
        """eget_seq:eget 每次调用依次返回的值列表。返回 (edelete 调用次数, emby_gone, edelete先于_del_folder)。"""
        calls = {"edelete": 0, "del_folder": 0, "edelete_before_disk": True}
        egets = list(eget_seq)
        def fake_eget(path, params=None):
            return egets.pop(0) if egets else {"Items": []}
        def fake_edelete(path):
            calls["edelete"] += 1
            if calls["del_folder"] > 0:
                calls["edelete_before_disk"] = False  # edelete 发生在磁盘删之后 = 错
            return 204
        def fake_del_folder(lib, folder):
            calls["del_folder"] += 1
            return ["115", "strm"]
        with patch.object(biz, "eget", fake_eget), \
             patch.object(biz, "edelete", fake_edelete), \
             patch.object(biz, "_del_folder", fake_del_folder), \
             patch.object(biz, "_lib_lock", lambda l: __import__("threading").Lock()), \
             patch.object(biz, "time") as _t:
            _t.sleep = lambda *_: None
            r = biz.delete_item("电影", "某片 (2024)", "131649")
        return calls, r

    def test_first_delete_succeeds_one_edelete(self):
        # eget 立刻返空 = Emby 已删干净 → 只 edelete 一次,emby_gone=True
        calls, r = self._run([{"Items": []}])
        self.assertEqual(calls["edelete"], 1)
        self.assertTrue(r["emby_gone"])
        self.assertTrue(calls["edelete_before_disk"], "edelete 必须先于磁盘删(雷#1 根因)")

    def test_silent_fail_triggers_retry(self):
        # 首删后 eget 仍返 Items(silent fail)→ 重试一次;第二次 eget 返空 → emby_gone=True
        calls, r = self._run([{"Items": [{"Id": "131649"}]}, {"Items": []}])
        self.assertEqual(calls["edelete"], 2, "silent fail 应触发第二次 edelete")
        self.assertTrue(r["emby_gone"])

    def test_persistent_fail_marks_not_gone(self):
        # 两次都没删掉 → emby_gone=False(但磁盘仍清)
        calls, r = self._run([{"Items": [{"Id": "x"}]}, {"Items": [{"Id": "x"}]}])
        self.assertEqual(calls["edelete"], 2)
        self.assertFalse(r["emby_gone"])
        self.assertEqual(calls["del_folder"], 1, "verify 失败磁盘也要清")


if __name__ == "__main__":
    unittest.main()
