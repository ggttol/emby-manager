"""_hash_password / _verify_password 测试:PBKDF2-HMAC-SHA256 配套验证。
不 patch iteration 数(200000)—— 单次 hash 仍在 < 1s 内,全部用例 < 2s 可接受。"""
import os, sys, unittest
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import app


class TestPassword(unittest.TestCase):

    def test_hash_then_verify_ok(self):
        h = app._hash_password("hunter2")
        self.assertTrue(app._verify_password("hunter2", h))

    def test_verify_wrong_password_fails(self):
        h = app._hash_password("hunter2")
        self.assertFalse(app._verify_password("hunter3", h))

    def test_verify_empty_plain_is_false(self):
        h = app._hash_password("anything")
        self.assertFalse(app._verify_password("", h))

    def test_verify_empty_stored_is_false(self):
        self.assertFalse(app._verify_password("anything", ""))

    def test_verify_both_empty_is_false(self):
        self.assertFalse(app._verify_password("", ""))

    def test_verify_garbage_stored_does_not_crash(self):
        self.assertFalse(app._verify_password("x", "not-a-hash-at-all"))
        self.assertFalse(app._verify_password("x", "pbkdf2_sha256$100"))  # 字段不够
        self.assertFalse(app._verify_password("x", "pbkdf2_sha256$abc$ff$ff"))  # iter 非数字
        self.assertFalse(app._verify_password("x", "wrong_scheme$200000$ff$ff"))

    def test_two_hashes_of_same_password_differ(self):
        # 盐应该真的随机
        h1 = app._hash_password("samepw")
        h2 = app._hash_password("samepw")
        self.assertNotEqual(h1, h2)
        # 但都应能 verify 通过
        self.assertTrue(app._verify_password("samepw", h1))
        self.assertTrue(app._verify_password("samepw", h2))

    def test_hash_format_is_pbkdf2_sha256(self):
        h = app._hash_password("x")
        parts = h.split("$")
        self.assertEqual(len(parts), 4)
        self.assertEqual(parts[0], "pbkdf2_sha256")
        self.assertEqual(parts[1], "200000")  # 当前默认迭代数
        # salt 是 hex 且长度 64(32 bytes)
        self.assertEqual(len(parts[2]), 64)
        int(parts[2], 16)  # 不该 throw — 验证是合法 hex
        int(parts[3], 16)

    def test_tampered_salt_fails(self):
        h = app._hash_password("hunter2")
        scheme, it, salt, hh = h.split("$")
        # 翻第一个 nybble
        bad_salt = ("0" if salt[0] != "0" else "1") + salt[1:]
        tampered = "$".join([scheme, it, bad_salt, hh])
        self.assertFalse(app._verify_password("hunter2", tampered))

    def test_tampered_hash_fails(self):
        h = app._hash_password("hunter2")
        scheme, it, salt, hh = h.split("$")
        bad_h = ("0" if hh[0] != "0" else "1") + hh[1:]
        tampered = "$".join([scheme, it, salt, bad_h])
        self.assertFalse(app._verify_password("hunter2", tampered))

    def test_tampered_iter_fails(self):
        # 改 iter → PBKDF2 出来的 hash 不同 → 必然 verify 失败
        h = app._hash_password("hunter2")
        scheme, it, salt, hh = h.split("$")
        tampered = "$".join([scheme, "100000", salt, hh])
        self.assertFalse(app._verify_password("hunter2", tampered))

    def test_unicode_password(self):
        # 中文密码必须 utf-8 编码,不能 crash
        h = app._hash_password("中文密码123")
        self.assertTrue(app._verify_password("中文密码123", h))
        self.assertFalse(app._verify_password("中文密码124", h))


if __name__ == "__main__":
    unittest.main()
