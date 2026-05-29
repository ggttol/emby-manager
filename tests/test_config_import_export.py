"""export_config / import_config 测试 — P0-2 regression。

P0-2:`import_config` 允许覆盖 `last_password_change_at` → 重启 grace 模式 → 绕过旧密码改密。
预期修复方向:`PROTECTED_IMPORT_KEYS`(扩展 SENSITIVE_KEYS)拒绝任意值 import,无论是不是 `<redacted>`。
"""
import os, sys, tempfile, unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

# 必须在 import lib.config 之前不能拦截,但 load_cfg() 会读 ./config.json。
# 安全做法:先 import,再把 CFG/CONFIG_FILE 切到隔离 tmpdir,后续测试都靠 reset_cfg 清状态。
import lib.config

TMPDIR = tempfile.mkdtemp(prefix="embymgr_imexp_")
lib.config.HERE = TMPDIR
lib.config.CONFIG_FILE = os.path.join(TMPDIR, "config.json")

from lib.config import CFG
from lib.business import export_config, import_config, SENSITIVE_KEYS
from lib.logger import AppError


def reset_cfg(**kw):
    """重置 CFG 到一个一致已知状态(给每个 case 用)。"""
    CFG.clear()
    CFG.update({
        "password_hash": "pbkdf2_sha256$200000$abcd$1234",
        "c115_cookie": "UID=real_uid_123; SEID=real_seid",
        "emby_url": "http://127.0.0.1:8096/emby",
        "api_key": "real_api_key",
        "port": 8097, "host": "127.0.0.1",
        "schema_version": 4,
        "last_password_change_at": 1700000000,
        "username": "admin",
        "trusted_proxies": [],
        "c115_cid_map": {"电影": "111"},
    })
    CFG.update(kw)


class ExportTests(unittest.TestCase):

    def test_redact_sensitive(self):
        reset_cfg()
        d = export_config()
        self.assertEqual(d["password_hash"], "<redacted>")
        self.assertEqual(d["c115_cookie"], "<redacted>")
        # 其他字段保留
        self.assertEqual(d["emby_url"], "http://127.0.0.1:8096/emby")
        self.assertEqual(d["api_key"], "real_api_key")
        self.assertEqual(d["last_password_change_at"], 1700000000)

    def test_redact_only_when_truthy(self):
        # 空 cookie/hash 不该被 redact(用户可能没设)
        reset_cfg(password_hash="", c115_cookie="")
        d = export_config()
        self.assertEqual(d["password_hash"], "")
        self.assertEqual(d["c115_cookie"], "")


class ImportTests(unittest.TestCase):

    def test_no_confirm_rejected(self):
        reset_cfg()
        with self.assertRaises(AppError) as cm:
            import_config({"cfg": {"emby_url": "http://new"}})
        self.assertEqual(cm.exception.status, 400)

    def test_non_dict_cfg_rejected(self):
        reset_cfg()
        with self.assertRaises(AppError) as cm:
            import_config({"cfg": "not-a-dict", "confirm": True})
        self.assertEqual(cm.exception.status, 400)

    def test_schema_mismatch_rejected(self):
        reset_cfg()
        with self.assertRaises(AppError) as cm:
            import_config({"cfg": {"schema_version": 1, "emby_url": "x"}, "confirm": True})
        self.assertEqual(cm.exception.status, 400)

    def test_schema_field_ignored_when_matching(self):
        reset_cfg()
        r = import_config({"cfg": {"schema_version": 4, "emby_url": "http://new"}, "confirm": True})
        self.assertTrue(r["ok"])
        # schema_version 字段本身不能被覆盖(始终是 PROTECTED)
        self.assertEqual(CFG["schema_version"], 4)
        self.assertEqual(CFG["emby_url"], "http://new")

    def test_redacted_kept_original(self):
        reset_cfg()
        import_config({"cfg": {"password_hash": "<redacted>", "c115_cookie": "<redacted>",
                              "emby_url": "http://new"}, "confirm": True})
        # 敏感字段保留原值
        self.assertEqual(CFG["password_hash"], "pbkdf2_sha256$200000$abcd$1234")
        self.assertEqual(CFG["c115_cookie"], "UID=real_uid_123; SEID=real_seid")
        self.assertEqual(CFG["emby_url"], "http://new")

    def test_normal_field_applied(self):
        reset_cfg()
        import_config({"cfg": {"emby_url": "http://nas:9999/emby",
                              "api_key": "newkey"}, "confirm": True})
        self.assertEqual(CFG["emby_url"], "http://nas:9999/emby")
        self.assertEqual(CFG["api_key"], "newkey")

    def test_host_and_trusted_proxies_protected(self):
        """host / trusted_proxies 是运行时安全开关,import 不接受(防恶意备份植入 0.0.0.0 / 伪 XFF 绕限流)"""
        reset_cfg()
        CFG["host"] = "127.0.0.1"; CFG["trusted_proxies"] = []
        r = import_config({"cfg": {"host": "0.0.0.0",
                                   "trusted_proxies": ["1.2.3.4"],
                                   "emby_url": "http://ok"}, "confirm": True})
        self.assertEqual(CFG["host"], "127.0.0.1")        # 没被改
        self.assertEqual(CFG["trusted_proxies"], [])       # 没被改
        self.assertEqual(CFG["emby_url"], "http://ok")     # 正常字段照应用
        self.assertIn("host", r["skipped_protected"])
        self.assertIn("trusted_proxies", r["skipped_protected"])

    # === 关键 P0-2 regression ===
    def test_last_password_change_at_protected(self):
        """P0-2 regression: import 不能把 last_password_change_at 改成 None 绕过 grace 改密保护"""
        reset_cfg()  # 当前 last_password_change_at = 1700000000(非 None)
        import_config({"cfg": {"last_password_change_at": None,
                              "emby_url": "http://new"}, "confirm": True})
        # 关键断言:last_password_change_at 必须保留原值(不能被绕过)
        self.assertEqual(CFG["last_password_change_at"], 1700000000)
        # 其他正常字段照样应用
        self.assertEqual(CFG["emby_url"], "http://new")

    def test_password_hash_protected_non_redacted(self):
        """非 <redacted> 的 password_hash 也不能被覆盖(防止攻击者直接换 hash)"""
        reset_cfg()
        orig_hash = CFG["password_hash"]
        import_config({"cfg": {"password_hash": "pbkdf2_sha256$200000$evil$evil",
                              "emby_url": "http://new"}, "confirm": True})
        self.assertEqual(CFG["password_hash"], orig_hash)  # 必须不变

    def test_c115_cookie_protected_non_redacted(self):
        """非 <redacted> 的 c115_cookie 不能直接覆盖(攻击者植入自己 cookie 偷数据)"""
        reset_cfg()
        orig = CFG["c115_cookie"]
        import_config({"cfg": {"c115_cookie": "UID=attacker; SEID=fake"}, "confirm": True})
        self.assertEqual(CFG["c115_cookie"], orig)

    def test_username_protected(self):
        """username 也保护(单用户场景影响小,但攻击面)"""
        reset_cfg()
        import_config({"cfg": {"username": "attacker"}, "confirm": True})
        self.assertEqual(CFG["username"], "admin")

    def test_unknown_keys_handling(self):
        """未知字段是否接受 — 当前实现接受(白名单可后续加强),测当前行为"""
        reset_cfg()
        import_config({"cfg": {"random_new_field": 42, "emby_url": "http://new"}, "confirm": True})
        self.assertEqual(CFG["emby_url"], "http://new")
        # 未知字段当前会写入,这条 case 记录现状


if __name__ == "__main__":
    unittest.main()
