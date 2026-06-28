"""任务队列语义：排队状态、开始前取消，以及运行中协作式取消。"""
import os
import sys
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


class TaskQueueTests(unittest.TestCase):
    def setUp(self):
        from lib import tasks
        with tasks.TASKS_LOCK:
            tasks.TASKS.clear()
        self.tasks = tasks
        self.slots = threading.BoundedSemaphore(1)
        self.slot_patch = patch.object(tasks, "_TASK_SLOTS", self.slots)
        self.slot_patch.start()

    def tearDown(self):
        self.slot_patch.stop()

    def _wait_for(self, predicate, msg="condition not met"):
        deadline = time.time() + 2
        while time.time() < deadline:
            if predicate():
                return
            time.sleep(0.01)
        self.fail(msg)

    def test_queued_task_is_pending_then_runs_when_slot_is_free(self):
        first_release = threading.Event()
        second_started = threading.Event()
        second_release = threading.Event()

        def first(_tid):
            first_release.wait(2)

        def second(_tid):
            second_started.set()
            second_release.wait(2)

        first_tid = self.tasks.run_async("first", first)
        self._wait_for(lambda: self.tasks.task_get(first_tid)["status"] == "running")
        first_task = self.tasks.task_get(first_tid)
        self.assertIsNotNone(first_task["queued_at"])
        self.assertIsNotNone(first_task["started_at"])
        self.assertGreaterEqual(first_task["started_at"], first_task["queued_at"])
        second_tid = self.tasks.run_async("second", second)
        self._wait_for(lambda: self.tasks.task_get(second_tid)["status"] == "pending")
        pending = self.tasks.task_get(second_tid)
        self.assertIsNotNone(pending["queued_at"])
        self.assertIsNone(pending["started_at"])
        self.assertFalse(second_started.is_set())

        first_release.set()
        self._wait_for(second_started.is_set)
        running = self.tasks.task_get(second_tid)
        self.assertEqual(running["status"], "running")
        self.assertIsNotNone(running["started_at"])
        self.assertGreaterEqual(running["started_at"], running["queued_at"])
        second_release.set()
        self._wait_for(lambda: self.tasks.task_get(second_tid)["status"] == "done")
        done = self.tasks.task_get(second_tid)
        self.assertIsNotNone(done["ended_at"])
        self.assertEqual(done["ended"], done["ended_at"])

    def test_cancelled_pending_task_never_calls_business_function(self):
        first_release = threading.Event()
        second_started = threading.Event()

        def first(_tid):
            first_release.wait(2)

        def second(_tid):
            second_started.set()

        first_tid = self.tasks.run_async("first", first)
        self._wait_for(lambda: self.tasks.task_get(first_tid)["status"] == "running")
        second_tid = self.tasks.run_async("second", second)
        self._wait_for(lambda: self.tasks.task_get(second_tid)["status"] == "pending")

        self.assertTrue(self.tasks.task_cancel(second_tid))
        cancelled = self.tasks.task_get(second_tid)
        self.assertEqual(cancelled["status"], "cancelled")
        self.assertIsNotNone(cancelled["ended"])

        first_release.set()
        time.sleep(0.1)
        self.assertFalse(second_started.is_set())
        self.assertEqual(self.tasks.task_get(second_tid)["status"], "cancelled")

    def test_running_task_stays_active_until_it_cooperatively_exits(self):
        release = threading.Event()

        def worker(tid):
            while not self.tasks.task_is_cancelled(tid):
                time.sleep(0.01)
            release.wait(2)

        tid = self.tasks.run_async("worker", worker)
        self._wait_for(lambda: self.tasks.task_get(tid)["status"] == "running")
        self.assertTrue(self.tasks.task_cancel(tid))
        running = self.tasks.task_get(tid)
        self.assertEqual(running["status"], "running")
        self.assertEqual(running["status_text"], "取消中…")

        release.set()
        self._wait_for(lambda: self.tasks.task_get(tid)["status"] == "cancelled")


if __name__ == "__main__":
    unittest.main()
