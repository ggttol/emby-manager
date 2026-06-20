"""ultracode 全面分析后修复的回归测试:挂载保险丝 / smart dst_n=0 护栏 /
qscore 剔样片 / save_cfg .bak 兜底 / update_user 回读 / undo undone 标记 / c115 snap 分页。
纯逻辑 + mock,不触网。"""
import os, sys, json, tempfile, threading, time, unittest
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

    def test_stuck_probe_does_not_spawn_more_probe_threads(self):
        """FUSE 卡住期间，后续探活直接失败，不能每次健康检查都堆一条 D-state 线程。"""
        started = threading.Event()
        release = threading.Event()
        calls = []

        def blocked_listdir(_path):
            calls.append(1)
            started.set()
            release.wait(1)
            return ["电影"]

        with tempfile.TemporaryDirectory() as cd, patch.object(biz, "CD", cd), \
             patch.object(biz.os, "listdir", side_effect=blocked_listdir):
            self.assertFalse(biz._mount_alive(timeout=0.02))
            self.assertTrue(started.is_set())
            self.assertFalse(biz._mount_alive(timeout=0.02))
            self.assertEqual(len(calls), 1)
            release.set()
            deadline = time.time() + 1
            while biz._MOUNT_PROBE_INFLIGHT and time.time() < deadline:
                time.sleep(0.01)
            self.assertFalse(biz._MOUNT_PROBE_INFLIGHT)
            self.assertTrue(biz._mount_alive(timeout=0.2))
            self.assertEqual(len(calls), 2)

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


class TestAsyncScannerFuse(unittest.TestCase):
    """review HIGH:scan_lib_async(手动扫描走的路径)也必须有挂载保险丝。"""
    def test_async_scanner_skips_orphan_when_mount_dead(self):
        with tempfile.TemporaryDirectory() as strm, tempfile.TemporaryDirectory() as cd:
            fol = "电影"
            d = os.path.join(strm, fol, "某片 (2024)"); os.makedirs(d)
            sp = os.path.join(d, "x.strm")
            with open(sp, "w") as f:
                f.write("/media/电影/某片 (2024)/x.mkv")
            fake_libs = {"电影": {"id": "1", "ctype": "movies", "folder": fol}}
            lock = __import__("threading").Lock()
            with patch.object(biz, "STRM", strm), patch.object(biz, "CD", cd), \
                 patch.object(biz, "fetch_libs", return_value=fake_libs), \
                 patch.object(biz, "_mount_alive", return_value=False), \
                 patch.object(biz, "_lib_lock", lambda l: lock), \
                 patch.object(biz, "epost", lambda *a, **k: 204), \
                 patch.object(biz, "eget", lambda *a, **k: {"Items": []}), \
                 patch.object(biz, "task_set", lambda *a, **k: None), \
                 patch.object(biz, "task_is_cancelled", lambda t: False):
                r = biz.scan_lib_async("tid1", "电影")
            self.assertTrue(os.path.exists(sp), "挂载死时 async 扫描也绝不能删 strm")
            self.assertEqual(r.get("orphans_cleaned", 0), 0)


class TestStrmPermissions(unittest.TestCase):
    """同类问题回归:STRM 存在但权限不可读时,扫描/移动必须修成 Emby 可读。"""
    def test_scan_repairs_existing_unreadable_strm(self):
        with tempfile.TemporaryDirectory() as strm, tempfile.TemporaryDirectory() as cd:
            fol = "纪录片"
            media_dir = os.path.join(cd, fol, "河西走廊 (2015)", "Season 1")
            strm_dir = os.path.join(strm, fol, "河西走廊 (2015)", "Season 1")
            os.makedirs(media_dir)
            os.makedirs(strm_dir)
            with open(os.path.join(media_dir, "E01.mp4"), "w") as f:
                f.write("x")
            sp = os.path.join(strm_dir, "E01.strm")
            with open(sp, "w") as f:
                f.write("/media/纪录片/河西走廊 (2015)/Season 1/E01.mp4")
            os.chmod(os.path.join(strm, fol, "河西走廊 (2015)"), 0o700)
            os.chmod(strm_dir, 0o700)
            os.chmod(sp, 0o600)
            fake_libs = {"纪录片": {"id": "1", "ctype": "tvshows", "folder": fol}}
            calls = {"refresh": 0}
            def fake_epost(*a, **k):
                calls["refresh"] += 1
                return 204
            with patch.object(biz, "STRM", strm), patch.object(biz, "CD", cd), \
                 patch.object(biz, "fetch_libs", return_value=fake_libs), \
                 patch.object(biz, "_mount_alive", return_value=True), \
                 patch.object(biz, "_lib_lock", lambda l: __import__("threading").Lock()), \
                 patch.object(biz, "epost", fake_epost):
                r = biz.scan_lib("纪录片")
            self.assertEqual(r.get("permissions_fixed"), 1)
            self.assertEqual(os.stat(os.path.join(strm, fol, "河西走廊 (2015)")).st_mode & 0o777, 0o755)
            self.assertEqual(os.stat(strm_dir).st_mode & 0o777, 0o755)
            self.assertEqual(os.stat(sp).st_mode & 0o777, 0o644)
            self.assertEqual(calls["refresh"], 1)

    def test_move_rebuilds_strm_with_readable_permissions_under_restrictive_umask(self):
        with tempfile.TemporaryDirectory() as strm, tempfile.TemporaryDirectory() as cd:
            ff, tf = "追更", "完结"
            src = os.path.join(cd, ff, "某剧", "Season 1")
            os.makedirs(src)
            with open(os.path.join(src, "E01.mp4"), "w") as f:
                f.write("x")
            L = {"追更": {"id": "1", "ctype": "tvshows", "folder": ff},
                 "完结": {"id": "2", "ctype": "tvshows", "folder": tf}}
            os.makedirs(os.path.join(cd, tf))
            old = os.umask(0o077)
            try:
                with patch.object(biz, "STRM", strm), patch.object(biz, "CD", cd), \
                     patch.object(biz, "epost", lambda *a, **k: 204), \
                     patch.object(biz, "edelete", lambda *a, **k: 204), \
                     patch.object(biz, "_undo_record", lambda *a, **k: None):
                    r = biz._move_item_locked("追更", "某剧", "完结", "eid", L)
            finally:
                os.umask(old)
            self.assertTrue(r.get("ok"))
            moved_dir = os.path.join(strm, tf, "某剧", "Season 1")
            sp = os.path.join(moved_dir, "E01.strm")
            self.assertEqual(os.stat(os.path.join(strm, tf, "某剧")).st_mode & 0o777, 0o755)
            self.assertEqual(os.stat(moved_dir).st_mode & 0o777, 0o755)
            self.assertEqual(os.stat(sp).st_mode & 0o777, 0o644)


class TestAddNewPipelineRetry(unittest.TestCase):
    """一条龙 receive 后 CloudDrive 延迟露出目录时,应短延迟补扫一次。"""
    def test_retries_scan_when_successful_receive_produces_no_strm(self):
        scans = [{"new_count": 0, "orphans_cleaned": 0, "matched": 1, "attention": []},
                 {"new_count": 3, "orphans_cleaned": 0, "matched": 2, "attention": []}]
        def fake_scan(_lib):
            return scans.pop(0)
        def fake_save(url, pwd, lib):
            return {"ok": True, "title": "片", "count": 1}
        with patch.object(biz, "scan_lib", fake_scan), \
             patch.object(biz, "list_noposter", return_value=[]), \
             patch.object(biz, "analyze_dups", return_value={"dups": [], "review": []}), \
             patch.object(biz, "task_set", lambda *a, **k: None), \
             patch.object(biz, "task_is_cancelled", lambda t: False), \
             patch.object(biz.time, "sleep", lambda *_: None):
            r = biz.add_new_pipeline_async("tid", [{"url": "u", "pwd": "", "lib": "电影"}], "", fake_save)
        self.assertEqual(r["libs_scanned"]["电影"]["new_count"], 3)
        self.assertTrue(r["libs_scanned"]["电影"]["retry"])


class TestSmartGuardSymmetric(unittest.TestCase):
    """review HIGH:smart 归档 src_n=0 也要拦(否则删源真实 115 内容)。"""
    def test_smart_refuses_when_src_strm_missing(self):
        with tempfile.TemporaryDirectory() as strm, tempfile.TemporaryDirectory() as cd:
            ff, tf = "追更", "完结"
            # 源 115 folder 存在但无 strm(src_n=0);目标 115 folder 存在且有 strm(dst_n>0)
            src = os.path.join(cd, ff, "某剧"); os.makedirs(src)
            dst = os.path.join(cd, tf, "某剧"); os.makedirs(dst)
            dstrm = os.path.join(strm, tf, "某剧"); os.makedirs(dstrm)
            with open(os.path.join(dstrm, "S01E01.strm"), "w") as f: f.write("/media/x")
            L = {"追更": {"id": "1", "ctype": "tvshows", "folder": ff},
                 "完结": {"id": "2", "ctype": "tvshows", "folder": tf}}
            with patch.object(biz, "STRM", strm), patch.object(biz, "CD", cd):
                r = biz._move_item_locked("追更", "某剧", "完结", "eid", L, on_conflict="smart")
            self.assertIn("err", r, "src strm 缺失时 smart 必须拒绝,不能删源")
            self.assertTrue(os.path.isdir(src), "源 115 内容不能被删")


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
        snap = dict(config.CFG)  # 快照,测完精确还原(load_cfg 会读真实磁盘污染其它测试的 CFG)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                cfgf = os.path.join(tmp, "config.json")
                good = {"emby_url": "http://good", "api_key": "K", "port": 8097,
                        "c115_cookie": "REAL_COOKIE", "password_hash": "h"}
                with open(cfgf, "w") as f: json.dump(good, f)
                with open(cfgf + ".bak", "w") as f: json.dump(good, f)
                with open(cfgf, "w") as f: f.write('{"emby_url": "http://good", BROKEN')
                with patch.object(config, "CONFIG_FILE", cfgf):
                    config.load_cfg()
                    self.assertEqual(config.CFG.get("c115_cookie"), "REAL_COOKIE", "应从 .bak 恢复")
                    self.assertTrue(config.CONFIG_EXISTED)
        finally:
            config.CFG.clear(); config.CFG.update(snap)

    def test_save_cfg_writes_bak(self):
        from lib import config
        snap = dict(config.CFG)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                cfgf = os.path.join(tmp, "config.json")
                with open(cfgf, "w") as f: json.dump({"v": 1}, f)
                with patch.object(config, "CONFIG_FILE", cfgf):
                    config.CFG.clear(); config.CFG.update({"v": 2})
                    config.save_cfg()
                    self.assertTrue(os.path.exists(cfgf + ".bak"))
                    with open(cfgf + ".bak") as f:
                        self.assertEqual(json.load(f).get("v"), 2, ".bak 应是刚写好的新配置,不是旧的")
        finally:
            config.CFG.clear(); config.CFG.update(snap)


class TestConfigurablePaths(unittest.TestCase):
    """CD/STRM/DOCKER 从 config.json 取(换机器不用改代码);缺 key 时回落默认。"""
    def test_defaults_when_absent(self):
        from lib import config
        snap = dict(config.CFG)
        try:
            for k in ("cd", "strm", "docker"):
                config.CFG.pop(k, None)
            config._apply_paths()
            self.assertEqual(config.CD, config._DEF_CD)
            self.assertEqual(config.STRM, config._DEF_STRM)
            self.assertEqual(config.DOCKER, config._DEF_DOCKER)
        finally:
            config.CFG.clear(); config.CFG.update(snap); config._apply_paths()

    def test_custom_paths_applied(self):
        from lib import config
        snap = dict(config.CFG)
        try:
            config.CFG["cd"] = "/mnt/115"; config.CFG["strm"] = "/mnt/strm"; config.CFG["docker"] = "/usr/bin/docker"
            config._apply_paths()
            self.assertEqual(config.CD, "/mnt/115")
            self.assertEqual(config.STRM, "/mnt/strm")
            self.assertEqual(config.DOCKER, "/usr/bin/docker")
        finally:
            config.CFG.clear(); config.CFG.update(snap); config._apply_paths()

    def test_set_config_rejects_relative_path(self):
        import lib.business as biz
        from lib import config
        snap = dict(config.CFG)
        try:
            r = biz.set_config({"cd": "relative/path"})
            self.assertIn("err", r)
            r2 = biz.set_config({"strm": "also/relative"})
            self.assertIn("err", r2)
        finally:
            config.CFG.clear(); config.CFG.update(snap); config._apply_paths()


if __name__ == "__main__":
    unittest.main()
