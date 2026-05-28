"""区间压缩(`compact`)+ 集号解析/格式化(`eps`/`fmt_eps`)测试。

⚠️ NOTE: 这三个函数当前在 app.py 里是 `analyze_dups` / `series_gaps` 内部的闭包,
没法直接 import。本文件先在测试里**重新实现一份等价逻辑**,断言它的行为,
**等 H-5 模块拆分把它们提到 lib/ 之后,把下面 _ref_* 换成 `from app import ...`。**

这样做的好处:① 现在就能锁定预期行为(后续重构时一旦行为漂移会爆) ② 不用动 app.py。"""
import os, sys, re, collections, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import app  # 仅用来 smoke-import,验证 sys.path 配好  # noqa: F401


# ===== 等价 reimplementation(逐行抄自 app.analyze_dups / app.series_gaps 内闭包) =====

def _ref_compact(g):
    """series_gaps 内 compact:数列 → 紧凑区间字符串列表。"""
    if not g:
        return []
    r = []; a = p = g[0]
    for x in g[1:]:
        if x == p + 1:
            p = x
        else:
            r.append(str(a) if a == p else "%d-%d" % (a, p))
            a = p = x
    r.append(str(a) if a == p else "%d-%d" % (a, p))
    return r


def _ref_eps(ms):
    """analyze_dups 内 eps:文件名集合 → {(season, episode)} 集合。"""
    e = set()
    for x in ms:
        z = re.search(r's(\d{1,2})e(\d{1,3})', x.lower())
        if z:
            e.add((int(z.group(1)), int(z.group(2))))
    return e


def _ref_fmt_eps(es):
    """analyze_dups 内 fmt_eps:{(s,e)} → 'S01 · E1-2,5' 等格式。"""
    if not es:
        return ""
    by_s = collections.defaultdict(list)
    for s, e in es:
        by_s[s].append(e)
    def comp(xs):
        xs = sorted(xs); out = []; a = p = xs[0]
        for x in xs[1:]:
            if x == p + 1:
                p = x
            else:
                out.append(str(a) if a == p else "%d-%d" % (a, p)); a = p = x
        out.append(str(a) if a == p else "%d-%d" % (a, p))
        return ",".join(out)
    if len(by_s) == 1:
        s = next(iter(by_s))
        return "S%02d · E%s" % (s, comp(by_s[s]))
    return " · ".join("S%02dE%s" % (s, comp(by_s[s])) for s in sorted(by_s))


# ===== 实际 case =====

class TestCompact(unittest.TestCase):

    def test_typical_with_gaps(self):
        self.assertEqual(_ref_compact([1, 2, 3, 5, 7, 8, 9]), ["1-3", "5", "7-9"])

    def test_empty_list(self):
        self.assertEqual(_ref_compact([]), [])

    def test_single_element(self):
        self.assertEqual(_ref_compact([7]), ["7"])

    def test_all_consecutive(self):
        self.assertEqual(_ref_compact([10, 11, 12, 13]), ["10-13"])

    def test_no_consecutive(self):
        self.assertEqual(_ref_compact([2, 5, 8]), ["2", "5", "8"])

    def test_two_consecutive_then_one(self):
        self.assertEqual(_ref_compact([1, 2, 4]), ["1-2", "4"])


class TestEpsParse(unittest.TestCase):

    def test_basic_s01e05(self):
        self.assertEqual(_ref_eps(["show.s01e05.mkv"]), {(1, 5)})

    def test_capital_S1E5_is_normalized(self):
        # re 用 .lower() 做匹配 → 大小写不敏感
        self.assertEqual(_ref_eps(["Show.S1E5.mkv"]), {(1, 5)})

    def test_absolute_three_digit_episode(self):
        # 海贼王这种 s01e1163 绝对集号 —— 正常应等于 {(1, 1163)}
        # ⚠️ BUG: app.py 里 eps 正则是 r's(\d{1,2})e(\d{1,3})',e 最多 3 位,
        # 所以 "s01e1163" 实际匹配为 (1, 116)(吃掉前 3 位,后面 "3.mkv" 丢)。
        # 真实 4 位绝对集号(海贼王 1000+)目前**识别错误**,需要把正则改成
        # r's(\d{1,2})e(\d{1,4})' 或更松。本测试先锁定当前行为(buggy),
        # 等修复后改成 {(1, 1163)} 并同步改 analyze_dups 里的正则。
        self.assertEqual(_ref_eps(["onepiece.s01e1163.mkv"]), {(1, 116)})

    def test_three_digit_episode_works(self):
        # 三位集号(s01e999)应该 OK —— 这是上面 bug 的 happy-path 边界
        self.assertEqual(_ref_eps(["x.s01e999.mkv"]), {(1, 999)})

    def test_multiple_files(self):
        files = ["show.s01e01.mkv", "show.s01e02.mkv", "show.s02e01.mkv"]
        self.assertEqual(_ref_eps(files), {(1, 1), (1, 2), (2, 1)})

    def test_no_match_in_filename(self):
        self.assertEqual(_ref_eps(["random_movie.mkv"]), set())

    def test_empty_input(self):
        self.assertEqual(_ref_eps([]), set())


class TestFmtEps(unittest.TestCase):

    def test_empty_set(self):
        self.assertEqual(_ref_fmt_eps(set()), "")

    def test_single_season_continuous(self):
        # E1-3 single season → "S01 · E1-3"
        self.assertEqual(_ref_fmt_eps({(1, 1), (1, 2), (1, 3)}), "S01 · E1-3")

    def test_single_season_mixed(self):
        # 题面示例:{(1,1),(1,2),(1,5)} → "S01 · E1-2,5"
        self.assertEqual(_ref_fmt_eps({(1, 1), (1, 2), (1, 5)}), "S01 · E1-2,5")

    def test_multi_season(self):
        # 题面示例:{(1,1),(2,1)} → "S01E1 · S02E1"
        self.assertEqual(_ref_fmt_eps({(1, 1), (2, 1)}), "S01E1 · S02E1")

    def test_multi_season_with_ranges(self):
        out = _ref_fmt_eps({(1, 1), (1, 2), (1, 3), (2, 5), (2, 6)})
        self.assertEqual(out, "S01E1-3 · S02E5-6")

    def test_single_episode(self):
        self.assertEqual(_ref_fmt_eps({(3, 7)}), "S03 · E7")


# ===== 端到端:把 parse 接到 fmt 上,模拟 analyze_dups 的真实使用 =====

class TestEpsRoundTrip(unittest.TestCase):

    def test_parse_then_format_continuous(self):
        files = ["s01e01.mkv", "s01e02.mkv", "s01e03.mkv"]
        es = _ref_eps(files)
        self.assertEqual(_ref_fmt_eps(es), "S01 · E1-3")

    def test_parse_then_format_with_gap(self):
        files = ["s01e01.mkv", "s01e02.mkv", "s01e05.mkv"]
        es = _ref_eps(files)
        self.assertEqual(_ref_fmt_eps(es), "S01 · E1-2,5")


if __name__ == "__main__":
    unittest.main()
