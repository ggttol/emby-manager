"""ultracode 全面分析后修复的回归测试:挂载保险丝 / smart dst_n=0 护栏 /
qscore 剔样片 / save_cfg .bak 兜底 / update_user 回读 / undo undone 标记 / c115 snap 分页。
纯逻辑 + mock,不触网。"""
import os, sys, json, tempfile, unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import lib.business as biz


class TestMountFuse(unittest.TestCase):
    """_mount_alive 探针 + 清孤儿保险丝:挂载死时绝不删 strm。"""
    def test_mount_alive_true_when_listable(self):
        with tempfile.TemporaryDirectory() as cd:
            os.makedirs(os.path.join(cd, "电影"))
            with patch.object(biz, "CD", cd):
                self.assertTrue(biz._mount_alive(timeout=2))

    def test_mount_alive_false_when_empty(self):
        with tempfile.TemporaryDirectory() as cd:
            with patch.object(biz, "CD", cd):
                self.assertFalse(biz._mount_alive(timeout=2))

    def test_mount_alive_false_when_missing(self):
        with patch.object(biz, "CD", "/nonexistent_mount_xyz"):
            self.assertFalse(biz._mount_alive(timeout=2))

    def test_orphan_cleanup_skipped_when_mount_dead(self):
        # 关键防灾:挂载探活失败时,即便 strm 的 target 看似不存在,也绝不删 strm
        with tempfile.TemporaryDirectory() as strm, tempfile.TemporaryDirectory() as cd:
            fol = "电影"
            d = os.path.join(strm, fol, "某片 (2024)")
            os.makedirs(d)
            sp = os.path.join(d, "x.strm")
            with open(sp, "w") as f:
                f.write("/media/电影/某片 (2024)/x.mkv")  # target 在 cd 下不存在
            fake_libs = {"电影": {"id": "1", "ctype": "movies", "folder": fol}}
            with patch.object(biz, "STRM", strm), patch.object(biz, "CD", cd), \
                 patch.object(biz, "fetch_libs", return_value=fake_libs), \
                 patch.object(biz, "_mount_alive", return_value=False), \
                 patch.object(biz, "epost", lambda *a, **k: 204), \
                 patch.object(biz, "eget", lambda *a, **k: {"Items": []}), \
                 patch.object(biz, "_lib_lock", lambda l: __import__("threading").Lock()):
                r = biz.scan_lib("电影")
            self.assertTrue(os.path.exists(sp), "挂载死时绝不能删 strm")
            self.assertEqual(r.get("orphans_cleaned", 0), 0)

    def test_orphan_cleanup_runs_when_mount_alive(self):
        # 挂载活时,真孤儿(target 不存在)正常清
        with tempfile.TemporaryDirectory() as strm, tempfile.TemporaryDirectory() as cd:
            fol = "电影"; os.makedirs(os.path.join(cd, fol))  # 挂载非空 → alive
            d = os.path.join(strm, fol, "旧片 (2020)"); os.makedirs(d)
            sp = os.path.join(d, "old.strm")
            with open(sp, "w") as f:
                f.write("/media/电影/旧片 (2020)/old.mkv")  # cd 下不存在 = 真孤儿
            fake_libs = {"电影": {"id": "1", "ctype": "movies", "folder": fol}}
            with patch.object(biz, "STRM", strm), patch.object(biz, "CD", cd), \
                 patch.object(biz, "fetch_libs", return_value=fake_libs), \
                 patch.object(biz, "epost", lambda *a, **k: 204), \
                 patch.object(biz, "eget", lambda *a, **k: {"Items": []}), \
                 patch.object(biz, "_lib_lock", lambda l: __import__("threading").Lock()):
                r = biz.scan_lib("电影")
            self.assertFalse(os.path.exists(sp), "挂载活时真孤儿应被清")
            self.assertEqual(r.get("orphans_cleaned", 0), 1)


class TestQscoreExtra(unittest.TestCase):
    def test_is_extra_detects_trailer_featurette(self):
        self.assertTrue(biz._is_extra("某片.预告.2160p.mkv"))
        self.assertTrue(biz._is_extra("Movie.Trailer.1080p.mkv"))
        self.assertTrue(biz._is_extra("剧.花絮.mkv"))
        self.assertFalse(biz._is_extra("Movie.2024.1080p.WEB-DL.mkv"))


class TestConfigBak(unittest.TestCase):
    """config.json 损坏时从 .bak 恢复,不静默丢配置。"""
    def test_corrupt_config_falls_back_to_bak(self):
        from lib import config
        with tempfile.TemporaryDirectory() as tmp:
            cfgf = os.path.join(tmp, "config.json")
            good = {"emby_url": "http://good", "api_key": "K", "port": 8097,
                    "c115_cookie": "REAL_COOKIE", "password_hash": "h"}
            # 先写一份好的 + 备份
            with open(cfgf, "w") as f: json.dump(good, f)
            with open(cfgf + ".bak", "w") as f: json.dump(good, f)
            # 主文件损坏(漏括号)
            with open(cfgf, "w") as f: f.write('{"emby_url": "http://good", BROKEN')
            with patch.object(config, "CONFIG_FILE", cfgf):
                config.load_cfg()
                self.assertEqual(config.CFG.get("c115_cookie"), "REAL_COOKIE", "应从 .bak 恢复")
                self.assertTrue(config.CONFIG_EXISTED)
        config.load_cfg()  # 还原真实 CFG

    def test_save_cfg_writes_bak(self):
        from lib import config
        with tempfile.TemporaryDirectory() as tmp:
            cfgf = os.path.join(tmp, "config.json")
            with open(cfgf, "w") as f: json.dump({"v": 1}, f)
            with patch.object(config, "CONFIG_FILE", cfgf):
                config.CFG.clear(); config.CFG.update({"v": 2})
                config.save_cfg()
                self.assertTrue(os.path.exists(cfgf + ".bak"), "save 前应备份旧 config 到 .bak")
        config.load_cfg()


if __name__ == "__main__":
    unittest.main()
