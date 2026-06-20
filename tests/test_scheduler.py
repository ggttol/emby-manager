"""scheduler 触发判定 / next_run 计算 / CRUD 测试。
不实际跑 _fire(那需要 emby/Tasks 全栈),只测纯逻辑 + CFG 持久化。"""
import os, sys, tempfile, unittest, threading
from datetime import datetime, timedelta
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


class _SchedulerTestBase(unittest.TestCase):
    """每个用例重置 CFG['schedules'],并用临时 CONFIG_FILE 路径防写真 config。"""

    @classmethod
    def setUpClass(cls):
        from lib import config
        cls._real_config_file = config.CONFIG_FILE
        cls._tmpdir = tempfile.mkdtemp(prefix="sched_test_")
        config.CONFIG_FILE = os.path.join(cls._tmpdir, "config.json")

    @classmethod
    def tearDownClass(cls):
        from lib import config
        config.CONFIG_FILE = cls._real_config_file
        import shutil
        shutil.rmtree(cls._tmpdir, ignore_errors=True)

    def setUp(self):
        from lib.config import CFG
        CFG["schedules"] = []


class TestSchedulerValidation(_SchedulerTestBase):
    def test_invalid_mode_rejected(self):
        from lib import scheduler
        with self.assertRaises(ValueError):
            scheduler.add_schedule("x", "scan_all", {"mode": "yearly", "hour": 3, "minute": 0})

    def test_hour_out_of_range_rejected(self):
        from lib import scheduler
        with self.assertRaises(ValueError):
            scheduler.add_schedule("x", "scan_all", {"mode": "daily", "hour": 25, "minute": 0})

    def test_weekday_out_of_range_rejected(self):
        from lib import scheduler
        with self.assertRaises(ValueError):
            scheduler.add_schedule("x", "scan_all", {"mode": "weekly", "hour": 3, "minute": 0, "weekday": 9})

    def test_day_out_of_range_rejected(self):
        from lib import scheduler
        with self.assertRaises(ValueError):
            scheduler.add_schedule("x", "scan_all", {"mode": "monthly", "hour": 3, "minute": 0, "day": 32})


class TestSchedulerCRUD(_SchedulerTestBase):
    def test_add_then_list(self):
        from lib import scheduler
        item = scheduler.add_schedule("nightly", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        self.assertTrue(item["id"].startswith("sch_"))
        self.assertEqual(item["name"], "nightly")
        rows = scheduler.list_schedules()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["kind"], "scan_all")

    def test_update_changes_fields(self):
        from lib import scheduler
        s = scheduler.add_schedule("a", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        u = scheduler.update_schedule(s["id"], {"name": "renamed", "enabled": False})
        self.assertEqual(u["name"], "renamed")
        self.assertFalse(u["enabled"])

    def test_update_nonexistent_returns_none(self):
        from lib import scheduler
        self.assertIsNone(scheduler.update_schedule("nope", {"name": "x"}))

    def test_update_invalid_schedule_rejected(self):
        from lib import scheduler
        s = scheduler.add_schedule("a", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        with self.assertRaises(ValueError):
            scheduler.update_schedule(s["id"], {"schedule": {"mode": "garbage", "hour": 1, "minute": 1}})

    def test_delete_removes(self):
        from lib import scheduler
        s = scheduler.add_schedule("a", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        self.assertTrue(scheduler.delete_schedule(s["id"]))
        self.assertFalse(scheduler.delete_schedule(s["id"]))  # idempotent → False
        self.assertEqual(scheduler.list_schedules(), [])


class TestIsDue(_SchedulerTestBase):
    """边界:5min 窗口、同周期防重入、模式匹配。"""

    def _mk(self, mode, **k):
        sch = {"mode": mode, "hour": 3, "minute": 0}
        sch.update(k)
        return {"id": "s1", "kind": "scan_all", "enabled": True,
                "schedule": sch, "last_run_at": None}

    def test_daily_in_window_fires(self):
        from lib import scheduler
        s = self._mk("daily")
        now = datetime(2026, 5, 28, 3, 2, 0)  # 03:02,窗口 [03:00, 03:05)
        self.assertTrue(scheduler.is_due(s, now))

    def test_daily_before_window_no_fire(self):
        from lib import scheduler
        s = self._mk("daily")
        now = datetime(2026, 5, 28, 2, 59, 30)
        self.assertFalse(scheduler.is_due(s, now))

    def test_daily_after_window_catches_up(self):
        # 新语义:去掉固定 5min 上限后,过点很久但本周期没跑过 → 仍补跑(防时钟跳变/轮询错拍漏跑)
        from lib import scheduler
        s = self._mk("daily")
        now = datetime(2026, 5, 28, 3, 10, 0)  # 过点 10 分钟,无 last_run_at
        self.assertTrue(scheduler.is_due(s, now))

    def test_daily_after_window_no_refire_if_ran_today(self):
        # 但当天已跑过就不再补跑(靠 last_run_at 同周期去重)
        from lib import scheduler
        s = self._mk("daily")
        s["last_run_at"] = "2026-05-28T03:00:00"
        now = datetime(2026, 5, 28, 3, 10, 0)
        self.assertFalse(scheduler.is_due(s, now))

    def test_daily_same_day_already_run_no_fire(self):
        from lib import scheduler
        s = self._mk("daily")
        s["last_run_at"] = "2026-05-28T03:00:00"
        now = datetime(2026, 5, 28, 3, 2, 0)
        self.assertFalse(scheduler.is_due(s, now))

    def test_daily_next_day_fires_again(self):
        from lib import scheduler
        s = self._mk("daily")
        s["last_run_at"] = "2026-05-28T03:00:00"
        now = datetime(2026, 5, 29, 3, 2, 0)
        self.assertTrue(scheduler.is_due(s, now))

    def test_weekly_correct_weekday_fires(self):
        from lib import scheduler
        s = self._mk("weekly", weekday=3)  # weekday 3 = Thursday
        now = datetime(2026, 5, 28, 3, 1, 0)  # 2026-5-28 是周四
        self.assertEqual(now.weekday(), 3)
        self.assertTrue(scheduler.is_due(s, now))

    def test_weekly_wrong_weekday_no_fire(self):
        from lib import scheduler
        s = self._mk("weekly", weekday=0)  # Monday
        now = datetime(2026, 5, 28, 3, 1, 0)  # Thursday
        self.assertFalse(scheduler.is_due(s, now))

    def test_monthly_correct_day_fires(self):
        from lib import scheduler
        s = self._mk("monthly", day=28)
        now = datetime(2026, 5, 28, 3, 1, 0)
        self.assertTrue(scheduler.is_due(s, now))

    def test_monthly_wrong_day_no_fire(self):
        from lib import scheduler
        s = self._mk("monthly", day=15)
        now = datetime(2026, 5, 28, 3, 1, 0)
        self.assertFalse(scheduler.is_due(s, now))


class TestNextRunDt(_SchedulerTestBase):
    def _mk(self, mode, **k):
        sch = {"mode": mode, "hour": 3, "minute": 0}
        sch.update(k)
        return {"id": "s1", "schedule": sch}

    def test_daily_today_future(self):
        from lib import scheduler
        s = self._mk("daily")
        now = datetime(2026, 5, 28, 2, 0, 0)
        nr = scheduler.next_run_dt(s, now)
        self.assertEqual(nr, datetime(2026, 5, 28, 3, 0, 0))

    def test_daily_today_past_returns_tomorrow(self):
        from lib import scheduler
        s = self._mk("daily")
        now = datetime(2026, 5, 28, 4, 0, 0)
        nr = scheduler.next_run_dt(s, now)
        self.assertEqual(nr, datetime(2026, 5, 29, 3, 0, 0))

    def test_weekly_future_same_week(self):
        from lib import scheduler
        s = self._mk("weekly", weekday=4)  # Friday
        now = datetime(2026, 5, 28, 4, 0, 0)  # Thursday past 03:00
        nr = scheduler.next_run_dt(s, now)
        self.assertEqual(nr, datetime(2026, 5, 29, 3, 0, 0))  # Friday 3am

    def test_weekly_same_day_past_returns_next_week(self):
        from lib import scheduler
        s = self._mk("weekly", weekday=3)  # Thursday
        now = datetime(2026, 5, 28, 4, 0, 0)  # Thursday past 03:00
        nr = scheduler.next_run_dt(s, now)
        self.assertEqual(nr, datetime(2026, 6, 4, 3, 0, 0))  # next Thursday

    def test_monthly_day_31_clamps_to_month_end(self):
        from lib import scheduler
        # 2026-02 only has 28 days
        s = self._mk("monthly", day=31)
        now = datetime(2026, 2, 1, 0, 0, 0)
        nr = scheduler.next_run_dt(s, now)
        self.assertEqual(nr.day, 28)
        self.assertEqual(nr.month, 2)


class TestSchedulesPersist(_SchedulerTestBase):
    """add_schedule 写完 save_cfg 后,文件里能看到。"""
    def test_add_persists_to_disk(self):
        import json
        from lib import scheduler
        from lib.config import CFG, CONFIG_FILE
        scheduler.add_schedule("a", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        with open(CONFIG_FILE) as f:
            saved = json.load(f)
        self.assertEqual(len(saved.get("schedules", [])), 1)
        self.assertEqual(saved["schedules"][0]["kind"], "scan_all")


class TestOverlapGuard(_SchedulerTestBase):
    """新守卫:只有上次任务【在内存 TASKS 里确实还 running】才短路;残留状态(重启/已终)不阻塞。"""

    def _set_status(self, sid, status, tid=None):
        from lib.config import CFG
        for x in CFG["schedules"]:
            if x["id"] == sid:
                x["last_status"] = status
                x["last_tid"] = tid

    def test_fire_skips_when_task_really_running(self):
        # last_tid 指向一个内存里确实 running 的任务 → _fire 必须短路,不起新任务
        from lib import scheduler
        from lib import tasks
        s = scheduler.add_schedule("ov", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        with patch.object(tasks, "task_get", return_value={"status": "running"}):
            self._set_status(s["id"], "running", tid="fake_running_tid")
            self.assertIsNone(scheduler._fire(s["id"]))

    def test_fire_skips_when_previous_task_is_queued(self):
        """排队任务也尚未结束，不能因为它还没取得 worker 槽就重入。"""
        from lib import scheduler
        from lib import tasks
        s = scheduler.add_schedule("queued", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        with patch.object(tasks, "task_get", return_value={"status": "pending"}):
            self._set_status(s["id"], "running", tid="fake_pending_tid")
            self.assertIsNone(scheduler._fire(s["id"]))

    def test_fire_proceeds_when_stale_running_after_restart(self):
        # H1:重启后 last_status 残留 running 但 task_get 返 None → 不应永久卡死,应能再次触发
        from lib import scheduler
        from lib import tasks
        captured = {}
        def fake_run_async(kind, fn, *a, **k):
            captured["kind"] = kind
            return "new_tid"
        s = scheduler.add_schedule("stale", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        with patch.object(tasks, "task_get", return_value=None), \
             patch.object(tasks, "run_async", side_effect=fake_run_async):
            self._set_status(s["id"], "running", tid="dead_tid_from_old_process")
            tid = scheduler._fire(s["id"])
            self.assertEqual(tid, "new_tid")
            self.assertEqual(captured.get("kind"), "schedule:scan_all")

    def test_reconcile_resets_stale_running(self):
        # 启动 reconcile:残留 running 且任务已不在 → 重置为 interrupted
        from lib import scheduler
        from lib import tasks
        from lib.config import CFG
        s = scheduler.add_schedule("rec", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        self._set_status(s["id"], "running", tid="dead")
        with patch.object(tasks, "task_get", return_value=None):
            scheduler._reconcile_on_start()
        got = scheduler.get_schedule(s["id"])
        self.assertEqual(got["last_status"], "interrupted")

    def test_update_does_not_change_kind(self):
        """update_schedule 只接受 name/params/schedule/enabled,kind 字段被忽略(防误改)。"""
        from lib import scheduler
        s = scheduler.add_schedule("k1", "scan_all", {"mode": "daily", "hour": 3, "minute": 0})
        # 故意传一个 kind 字段(routing 层会过滤,直接调 update 也不能改)
        u = scheduler.update_schedule(s["id"], {"kind": "fix_posters_all", "name": "renamed"})
        self.assertEqual(u["kind"], "scan_all")    # 没被改
        self.assertEqual(u["name"], "renamed")     # name 改了


class TestMonitorIncrementalKind(_SchedulerTestBase):
    """autostrm 增量补扫调度 kind 注册 + fn 签名(_fire 只传 tid)。"""

    def test_monitor_incremental_registered(self):
        from lib.business import SCHEDULE_KINDS
        self.assertIn("monitor_incremental", SCHEDULE_KINDS)
        self.assertTrue(callable(SCHEDULE_KINDS["monitor_incremental"]["fn"]))

    def test_monitor_incremental_fn_arity_is_tid_only(self):
        # _fire 调 run_async("schedule:"+kind, fn) → fn(tid)。监控 fn 只能收 tid(其余读 CFG)。
        import inspect
        from lib.business import monitor_incremental_async
        self.assertEqual(list(inspect.signature(monitor_incremental_async).parameters), ["tid"])

    def test_can_add_schedule_with_kind(self):
        from lib import scheduler
        s = scheduler.add_schedule("夜间增量补扫", "monitor_incremental",
                                   {"mode": "daily", "hour": 4, "minute": 0})
        self.assertEqual(s["kind"], "monitor_incremental")


if __name__ == "__main__":
    unittest.main()
