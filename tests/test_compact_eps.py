"""区间压缩 + 集号解析/格式化测试。"""
import os, sys, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from lib.dedup import compact_ints, episode_set, fmt_eps


# ===== 实际 case =====

class TestCompact(unittest.TestCase):

    def test_typical_with_gaps(self):
        self.assertEqual(compact_ints([1, 2, 3, 5, 7, 8, 9]), ["1-3", "5", "7-9"])

    def test_empty_list(self):
        self.assertEqual(compact_ints([]), [])

    def test_single_element(self):
        self.assertEqual(compact_ints([7]), ["7"])

    def test_all_consecutive(self):
        self.assertEqual(compact_ints([10, 11, 12, 13]), ["10-13"])

    def test_no_consecutive(self):
        self.assertEqual(compact_ints([2, 5, 8]), ["2", "5", "8"])

    def test_two_consecutive_then_one(self):
        self.assertEqual(compact_ints([1, 2, 4]), ["1-2", "4"])


class TestEpsParse(unittest.TestCase):

    def test_basic_s01e05(self):
        self.assertEqual(episode_set(["show.s01e05.mkv"]), {(1, 5)})

    def test_capital_S1E5_is_normalized(self):
        # re 用 .lower() 做匹配 → 大小写不敏感
        self.assertEqual(episode_set(["Show.S1E5.mkv"]), {(1, 5)})

    def test_absolute_three_digit_episode(self):
        self.assertEqual(episode_set(["onepiece.s01e1163.mkv"]), {(1, 1163)})

    def test_three_digit_episode_works(self):
        # 三位集号(s01e999)应该 OK —— 这是上面 bug 的 happy-path 边界
        self.assertEqual(episode_set(["x.s01e999.mkv"]), {(1, 999)})

    def test_multiple_files(self):
        files = ["show.s01e01.mkv", "show.s01e02.mkv", "show.s02e01.mkv"]
        self.assertEqual(episode_set(files), {(1, 1), (1, 2), (2, 1)})

    def test_no_match_in_filename(self):
        self.assertEqual(episode_set(["random_movie.mkv"]), set())

    def test_empty_input(self):
        self.assertEqual(episode_set([]), set())


class TestFmtEps(unittest.TestCase):

    def test_empty_set(self):
        self.assertEqual(fmt_eps(set()), "")

    def test_single_season_continuous(self):
        # E1-3 single season → "S01 · E1-3"
        self.assertEqual(fmt_eps({(1, 1), (1, 2), (1, 3)}), "S01 · E1-3")

    def test_single_season_mixed(self):
        # 题面示例:{(1,1),(1,2),(1,5)} → "S01 · E1-2,5"
        self.assertEqual(fmt_eps({(1, 1), (1, 2), (1, 5)}), "S01 · E1-2,5")

    def test_multi_season(self):
        # 题面示例:{(1,1),(2,1)} → "S01E1 · S02E1"
        self.assertEqual(fmt_eps({(1, 1), (2, 1)}), "S01E1 · S02E1")

    def test_multi_season_with_ranges(self):
        out = fmt_eps({(1, 1), (1, 2), (1, 3), (2, 5), (2, 6)})
        self.assertEqual(out, "S01E1-3 · S02E5-6")

    def test_single_episode(self):
        self.assertEqual(fmt_eps({(3, 7)}), "S03 · E7")


# ===== 端到端:把 parse 接到 fmt 上,模拟 analyze_dups 的真实使用 =====

class TestEpsRoundTrip(unittest.TestCase):

    def test_parse_then_format_continuous(self):
        files = ["s01e01.mkv", "s01e02.mkv", "s01e03.mkv"]
        es = episode_set(files)
        self.assertEqual(fmt_eps(es), "S01 · E1-3")

    def test_parse_then_format_with_gap(self):
        files = ["s01e01.mkv", "s01e02.mkv", "s01e05.mkv"]
        es = episode_set(files)
        self.assertEqual(fmt_eps(es), "S01 · E1-2,5")


if __name__ == "__main__":
    unittest.main()
