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

    def test_catalog_results_escape_untrusted_fields(self):
        html = INDEX.read_text(encoding="utf-8")
        self.assertIn("${esc(it.name)}", html)
        self.assertIn("${esc(it.sheet)}", html)
        self.assertIn("${esc(plain(it.link).slice(0,46))}", html)
        self.assertNotIn("${it.name}", html)
        self.assertNotIn("${it.sheet}", html)
        self.assertNotIn("${it.link}", html)

    def test_c115_batch_preview_escapes_untrusted_fields(self):
        html = INDEX.read_text(encoding="utf-8")
        self.assertIn("${esc(x.title||x.share)}", html)
        self.assertIn("${esc(plain(x.url).slice(0,60))}", html)
        self.assertIn("${esc(x.err||'')}", html)
        self.assertNotIn("${x.title||x.share}", html)
        self.assertNotIn("${x.url}", html)
        self.assertNotIn("${x.err}", html)

    def test_modal_html_escape_contract_is_explicit(self):
        html = INDEX.read_text(encoding="utf-8")
        self.assertIn("body.innerHTML = bodyNode;  // 调用方负责 esc", html)
        self.assertIn("bodyWrap.innerHTML = body;  // 允许 HTML(调用者负责安全)", html)

    def test_task_center_hydrates_backend_timestamps(self):
        html = INDEX.read_text(encoding="utf-8")
        self.assertIn("queued_at", html)
        self.assertIn("started_at", html)
        self.assertIn("ended_at", html)
        self.assertIn("updated_at", html)
        self.assertNotIn("created_at", html)

    def test_task_center_running_cancel_keeps_polling(self):
        html = INDEX.read_text(encoding="utf-8")
        cancel_start = html.index("async function cancel")
        cancel_end = html.index("function clearFinished")
        cancel_body = html[cancel_start:cancel_end]
        self.assertIn("T[tid].status_text = '取消中…';", cancel_body)
        self.assertIn("_poll(tid);", cancel_body)

    def test_task_center_drawer_has_refresh_and_clear_actions(self):
        html = INDEX.read_text(encoding="utf-8")
        self.assertIn("data-refresh-tasks", html)
        self.assertIn("data-clear-finished", html)
        self.assertIn("clearFinished", html)

    def test_task_center_bell_click_binding_is_not_duplicated(self):
        html = INDEX.read_text(encoding="utf-8")
        self.assertIn("__tasksBellBound", html)
        self.assertNotIn("bellBtn.addEventListener('click'", html)


if __name__ == "__main__":
    unittest.main()
