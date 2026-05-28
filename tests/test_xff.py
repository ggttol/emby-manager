import os, sys, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from lib.auth import client_ip_for_login

class XFFTests(unittest.TestCase):
    def test_no_trusted_returns_remote(self):
        # 不配信任反代 → 任何 XFF 都不读
        self.assertEqual(client_ip_for_login("1.2.3.4", "", []), "1.2.3.4")
        self.assertEqual(client_ip_for_login("1.2.3.4", "8.8.8.8", []), "1.2.3.4")

    def test_trusted_proxy_reads_xff(self):
        self.assertEqual(client_ip_for_login("192.168.2.1", "8.8.8.8", ["192.168.2.1"]), "8.8.8.8")

    def test_untrusted_remote_ignores_xff(self):
        # 直连不在信任列表 → XFF 不可信
        self.assertEqual(client_ip_for_login("1.2.3.4", "8.8.8.8", ["192.168.2.1"]), "1.2.3.4")

    def test_strip_trusted_from_right(self):
        # 链最右是 trusted 自己,剥掉取下一个
        self.assertEqual(
            client_ip_for_login("192.168.2.1", "1.1.1.1, 2.2.2.2, 192.168.2.1", ["192.168.2.1"]),
            "2.2.2.2")

    def test_take_rightmost_real_client(self):
        # 多层链:最右非 trusted 的 IP 才是直连本最近的 client
        self.assertEqual(
            client_ip_for_login("192.168.2.1", "1.1.1.1, 2.2.2.2", ["192.168.2.1"]),
            "2.2.2.2")

    def test_multiple_trusted_proxies(self):
        # 两层反代都信任
        self.assertEqual(
            client_ip_for_login("10.0.0.1", "8.8.8.8, 192.168.2.1, 10.0.0.1",
                                ["10.0.0.1", "192.168.2.1"]),
            "8.8.8.8")

    def test_all_trusted_in_chain(self):
        # 链里全是 trusted(配置异常)→ 退化到 remote
        self.assertEqual(
            client_ip_for_login("192.168.2.1", "10.0.0.1, 192.168.2.1", ["10.0.0.1", "192.168.2.1"]),
            "192.168.2.1")

    def test_empty_remote(self):
        self.assertEqual(client_ip_for_login("", "8.8.8.8", ["192.168.2.1"]), "?")
        self.assertEqual(client_ip_for_login(None, "", []), "?")

    def test_xff_whitespace_handling(self):
        # 不规范空格
        self.assertEqual(
            client_ip_for_login("192.168.2.1", "  8.8.8.8  ,  9.9.9.9  ", ["192.168.2.1"]),
            "9.9.9.9")

    def test_empty_xff_with_trusted_remote(self):
        self.assertEqual(client_ip_for_login("192.168.2.1", "", ["192.168.2.1"]), "192.168.2.1")

if __name__ == "__main__":
    unittest.main()
