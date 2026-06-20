import importlib.util
import os
import sqlite3
import tempfile
import unittest


ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(ROOT, "scripts", "validate_catalog_115_links.py")
spec = importlib.util.spec_from_file_location("validate_catalog_115_links", SCRIPT)
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


class TestValidateCatalog115Links(unittest.TestCase):
    def test_parse_115_url(self):
        self.assertEqual(
            validator.parse_115_url("https://115.com/s/swabc123?password=8888&"),
            ("swabc123", "8888"),
        )
        self.assertEqual(validator.parse_115_url("swabc123 9999"), ("swabc123", "9999"))

    def test_iter_catalog_shares_dedupes_link_pairs(self):
        with tempfile.TemporaryDirectory() as tmp:
            db = os.path.join(tmp, "catalog_115.db")
            con = sqlite3.connect(db)
            con.execute("CREATE TABLE catalog(name TEXT, sheet TEXT, link TEXT, is_pkg INT, link_type TEXT)")
            rows = [
                ("a", "s", "https://115.com/s/swabc123?password=8888&", 0, "share115"),
                ("a dup", "s", "https://115.com/s/swabc123?password=8888&#", 0, "share115"),
                ("b", "s", "magnet:?xt=urn:btih:abc", 0, "magnet"),
                ("c", "s", "https://115.com/s/swxyz789?password=9999", 0, "share115"),
            ]
            con.executemany("INSERT INTO catalog VALUES (?,?,?,?,?)", rows)
            con.commit()
            con.close()

            self.assertEqual(
                list(validator.iter_catalog_shares(db)),
                [("swabc123", "8888"), ("swxyz789", "9999")],
            )

    def test_classify_snap(self):
        ok = validator.classify_snap({
            "state": True,
            "data": {"shareinfo": {"share_title": "demo", "file_count": 3}, "list": [{}]},
        })
        self.assertEqual(ok["status"], "ok")
        self.assertEqual(ok["file_count"], 3)

        bad = validator.classify_snap({"state": False, "error": "分享不存在"})
        self.assertEqual(bad["status"], "invalid")
        self.assertFalse(bad["ok"])


if __name__ == "__main__":
    unittest.main()
