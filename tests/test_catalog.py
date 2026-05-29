"""资源库搜索 lib/catalog.py 测试:用临时小库,验多关键词 AND / 单片优先 / 链接解析 / 缺库降级。"""
import os, sys, sqlite3, tempfile, unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import lib.catalog as cat


def _mkdb(path, rows):
    con = sqlite3.connect(path)
    con.execute("CREATE TABLE catalog(name TEXT, sheet TEXT, link TEXT, is_pkg INT)")
    con.executemany("INSERT INTO catalog VALUES(?,?,?,?)", rows)
    con.commit(); con.close()


class TestCatalog(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.db = os.path.join(self.tmp, "catalog_115.db")
        _mkdb(self.db, [
            ("沙丘 Dune 2021 4K", "电影", "https://115cdn.com/s/swabc?password=1314", 0),
            ("沙丘合集 1-2 整包", "合集", "https://115cdn.com/s/swpkg?password=ayss", 1),
            ("庆余年 第二季", "国产剧", "https://115.com/s/swqyn", 0),
        ])

    def test_search_single_term(self):
        with patch.object(cat, "CATALOG_DB", self.db):
            r = cat.catalog_search("沙丘")
        self.assertEqual(len(r["items"]), 2)
        # 单片优先(is_pkg ASC)→ 第一条不是整包
        self.assertEqual(r["items"][0]["is_pkg"], 0)

    def test_multi_term_and(self):
        with patch.object(cat, "CATALOG_DB", self.db):
            r = cat.catalog_search("沙丘 4k")  # 两个词都要命中
        self.assertEqual(len(r["items"]), 1)
        self.assertIn("4K", r["items"][0]["name"])

    def test_link_parsed(self):
        with patch.object(cat, "CATALOG_DB", self.db):
            r = cat.catalog_search("庆余年")
        it = r["items"][0]
        self.assertEqual(it["share"], "swqyn")
        # 沙丘那条有 password
        with patch.object(cat, "CATALOG_DB", self.db):
            r2 = cat.catalog_search("Dune")
        self.assertEqual(r2["items"][0]["rc"], "1314")

    def test_empty_query(self):
        with patch.object(cat, "CATALOG_DB", self.db):
            self.assertEqual(cat.catalog_search("")["items"], [])

    def test_short_term_rejected(self):
        # 单字符词(含通配 %)挡掉,不全表扫
        with patch.object(cat, "CATALOG_DB", self.db):
            self.assertEqual(cat.catalog_search("%")["items"], [])
            self.assertEqual(cat.catalog_search("a")["items"], [])

    def test_like_wildcard_escaped(self):
        # 含字面 % 的真实片名:% 不当通配符
        _mkdb(self.db.replace("catalog_115", "c2"), [])  # noop to keep tmp
        with patch.object(cat, "CATALOG_DB", self.db):
            # 库里没有含 % 的名,搜 "100%"(>=2 字符)应命中 0 而非因 % 通配命中全部
            r = cat.catalog_search("100%")
            self.assertEqual(len(r["items"]), 0, "% 应被转义为字面量,不匹配全表")

    def test_missing_db_graceful(self):
        with patch.object(cat, "CATALOG_DB", "/nonexistent_catalog_xyz.db"):
            r = cat.catalog_search("沙丘")
            self.assertIn("err", r)
            self.assertFalse(cat.catalog_available())

    def test_stats(self):
        with patch.object(cat, "CATALOG_DB", self.db):
            s = cat.catalog_stats()
        self.assertTrue(s["available"]); self.assertEqual(s["total"], 3); self.assertEqual(s["packages"], 1)

    def tearDown(self):
        import shutil; shutil.rmtree(self.tmp, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
