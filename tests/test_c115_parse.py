"""c115_parse_url 测试:解析 115 分享链接为 (share_code, receive_code)。
覆盖多域名 / 多分隔符 / 显式 pwd 覆盖 / 空输入等。"""
import os, sys, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import app


class TestC115ParseUrl(unittest.TestCase):

    def test_115_com_with_password(self):
        s, r = app.c115_parse_url("https://115.com/s/swABC?password=YYY")
        self.assertEqual(s, "swABC")
        self.assertEqual(r, "YYY")

    def test_115cdn_with_password(self):
        s, r = app.c115_parse_url("https://115cdn.com/s/swXYZ?password=ABC")
        self.assertEqual(s, "swXYZ")
        self.assertEqual(r, "ABC")

    def test_anxia_without_password(self):
        s, r = app.c115_parse_url("https://anxia.com/s/swDEF")
        self.assertEqual(s, "swDEF")
        self.assertIsNone(r)

    def test_space_separated_form(self):
        s, r = app.c115_parse_url("swABC YYY")
        self.assertEqual(s, "swABC")
        self.assertEqual(r, "YYY")

    def test_comma_separated_form(self):
        s, r = app.c115_parse_url("swABC,YYY")
        self.assertEqual(s, "swABC")
        self.assertEqual(r, "YYY")

    def test_bare_share_code(self):
        s, r = app.c115_parse_url("swABC")
        self.assertEqual(s, "swABC")
        self.assertIsNone(r)

    def test_explicit_pwd_overrides_url_password(self):
        # 显式 pwd 参数应该覆盖 URL 内的 password
        s, r = app.c115_parse_url("https://115.com/s/swABC?password=URLPW", pwd="OVERRIDE")
        self.assertEqual(s, "swABC")
        self.assertEqual(r, "OVERRIDE")

    def test_empty_string(self):
        self.assertEqual(app.c115_parse_url(""), (None, None))

    def test_none_url(self):
        # 函数对 None 也要 robust(.strip() 前要么转 str 要么默认空)
        self.assertEqual(app.c115_parse_url(None), (None, None))

    def test_whitespace_only(self):
        self.assertEqual(app.c115_parse_url("   "), (None, None))

    def test_garbage_input(self):
        # 既无 /s/ 又无 alphanumeric token → 返回 (None, None)
        self.assertEqual(app.c115_parse_url("!!!@@@###"), (None, None))

    def test_pwd_param_with_fragment_in_url(self):
        # password=YYY 后面带 # 锚点不应混入 receive_code
        s, r = app.c115_parse_url("https://115.com/s/swABC?password=YYY#anchor")
        self.assertEqual(s, "swABC")
        self.assertEqual(r, "YYY")

    def test_url_with_pwd_param_alias(self):
        # 兼容 ?pwd= 别名
        s, r = app.c115_parse_url("https://115.com/s/swABC?pwd=YYY")
        self.assertEqual(s, "swABC")
        self.assertEqual(r, "YYY")

    def test_strips_outer_whitespace(self):
        s, r = app.c115_parse_url("   https://115.com/s/swABC?password=YYY   ")
        self.assertEqual(s, "swABC")
        self.assertEqual(r, "YYY")


if __name__ == "__main__":
    unittest.main()
