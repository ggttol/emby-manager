import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
INDEX = ROOT / "index.html"


class FrontendStaticTests(unittest.TestCase):
    def test_global_escape_stringifies_non_strings(self):
        html = INDEX.read_text(encoding="utf-8")
        esc_defs = re.findall(r"function\s+esc\s*\([^)]*\)\s*\{([^{}]*(?:\{[^{}]*\}[^{}]*)*)\}", html)
        self.assertTrue(esc_defs)
        self.assertNotIn("(s||\"\").replace", html)
        self.assertNotIn("(s||'').replace", html)
        self.assertRegex(esc_defs[-1], r"plain\(s\)\.replace|String\(s\).*\.replace")


if __name__ == "__main__":
    unittest.main()
