"""qscore 测试:验证文件名画质打分的相对排序,
不锁定绝对数值(具体加分可能微调)。"""
import os, sys, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import app


class TestQScore(unittest.TestCase):

    def test_empty_string_is_zero(self):
        self.assertEqual(app.qscore(""), 0)

    def test_no_quality_keywords_is_zero(self):
        self.assertEqual(app.qscore("some.random.movie.title.mkv"), 0)

    def test_2160p_beats_1080p_beats_720p_beats_dvdrip(self):
        a = app.qscore("Show.2160p.WEB-DL.mkv")
        b = app.qscore("Show.1080p.WEB-DL.mkv")
        c = app.qscore("Show.720p.WEB-DL.mkv")
        d = app.qscore("Show.DVDRip.WEB-DL.mkv")
        self.assertGreater(a, b)
        self.assertGreater(b, c)
        self.assertGreater(c, d)

    def test_4k_chinese_label_counts_as_2160(self):
        s = app.qscore("某剧 4K 高清版.mkv")
        # 至少要拿到 2160p 那档(4000),不能是 0
        self.assertGreaterEqual(s, 1000)

    def test_remux_beats_bluray_beats_webdl_beats_hdtv(self):
        base = "Show.1080p"
        rmx = app.qscore(base + ".REMUX.mkv")
        blu = app.qscore(base + ".BluRay.mkv")
        web = app.qscore(base + ".WEB-DL.mkv")
        hdtv = app.qscore(base + ".HDTV.mkv")
        self.assertGreater(rmx, blu)
        self.assertGreater(blu, web)
        self.assertGreater(web, hdtv)

    def test_hdr_and_dovi_bonus_stacks(self):
        plain = app.qscore("Show.1080p.WEB-DL.mkv")
        hdr = app.qscore("Show.1080p.WEB-DL.HDR.mkv")
        dovi = app.qscore("Show.1080p.WEB-DL.HDR.DV.mkv")
        self.assertGreater(hdr, plain)
        self.assertGreater(dovi, hdr)

    def test_case_insensitive(self):
        # qscore 小写化处理 → 大小写不应该影响打分
        a = app.qscore("Show.2160P.REMUX.MKV")
        b = app.qscore("show.2160p.remux.mkv")
        self.assertEqual(a, b)

    def test_chinese_dolby_vision_label(self):
        a = app.qscore("某剧.1080p.WEB-DL.mkv")
        b = app.qscore("某剧.1080p.WEB-DL.杜比视界.mkv")
        self.assertGreater(b, a)

    def test_best_combo_2160_remux_hdr_dv_is_highest(self):
        top = app.qscore("Movie.2160p.REMUX.HDR.DV.mkv")
        others = [
            app.qscore("Movie.1080p.REMUX.HDR.DV.mkv"),
            app.qscore("Movie.2160p.WEB-DL.HDR.mkv"),
            app.qscore("Movie.720p.BluRay.mkv"),
            app.qscore("Movie.1080p.HDTV.mkv"),
            app.qscore("Movie.480p.DVDRip.mkv"),
            app.qscore("Movie.mkv"),
        ]
        for o in others:
            self.assertGreater(top, o)

    def test_returns_int(self):
        # 排序键依赖 int,别意外回 float
        self.assertIsInstance(app.qscore("any"), int)


if __name__ == "__main__":
    unittest.main()
