import importlib.util
import os
import unittest


ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(ROOT, "scripts", "update_catalog_from_qqdocs.py")
spec = importlib.util.spec_from_file_location("update_catalog_from_qqdocs", SCRIPT)
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)


class TestUpdateCatalogFromQQDocs(unittest.TestCase):
    def test_normalize_wrapped_magnet_and_ed2k_links(self):
        self.assertEqual(
            updater.normalize_link("https://magnet:?xt=urn:btih:abc&amp;dn=x"),
            "magnet:?xt=urn:btih:abc&dn=x",
        )
        self.assertEqual(
            updater.normalize_link("https://ed2k://|file|demo.mkv|1|hash|/"),
            "ed2k://|file|demo.mkv|1|hash|/",
        )

    def test_normalize_rich_text_name_prefix(self):
        self.assertEqual(
            updater.normalize_name_text("q玩爆约会[无字片源].Playdate.2025.1080p.WEB-DL"),
            "玩爆约会[无字片源].Playdate.2025.1080p.WEB-DL",
        )
        self.assertEqual(
            updater.normalize_name_text("* FF000000 斗破苍穹.年番1[第149集][中文字幕]"),
            "斗破苍穹.年番1[第149集][中文字幕]",
        )


if __name__ == "__main__":
    unittest.main()
