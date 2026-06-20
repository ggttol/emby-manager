"""持久化操作日志：服务重启后日志页仍能看到最近记录。"""
import os
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


class PersistentLogTests(unittest.TestCase):
    def test_recent_logs_returns_file_tail_newest_first(self):
        from lib import logger
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "app.log")
            with open(path, "w", encoding="utf-8") as f:
                for i in range(8):
                    f.write("line-%d\n" % i)
            with patch.object(logger, "LOG_FILE", path):
                self.assertEqual(logger.list_recent_logs(3), ["line-7", "line-6", "line-5"])

    def test_recent_logs_clamps_limit_and_handles_bad_value(self):
        from lib import logger
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "app.log")
            with open(path, "w", encoding="utf-8") as f:
                f.write("only\n")
            with patch.object(logger, "LOG_FILE", path):
                self.assertEqual(logger.list_recent_logs("bad"), ["only"])
                self.assertEqual(logger.list_recent_logs(0), ["only"])


if __name__ == "__main__":
    unittest.main()
