"""追更异步检查：任务结果必须可直接驱动前端的归档操作。"""
import os
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


class ZhuigengAsyncTests(unittest.TestCase):
    def test_async_result_keeps_id_and_folder_for_archive(self):
        from lib import business
        calls = []

        def fake_eget(path, params=None):
            calls.append((path, params or {}))
            if path == "/Items":
                return {"Items": [{"Id": "series-1", "Name": "示例剧", "Status": "Ended",
                                   "Path": "/strm/追更剧/示例剧 [tmdbid-1]/Season 1/E01.strm",
                                   "ProviderIds": {"Tmdb": "1"}}]}
            if path == "/Shows/series-1/Episodes":
                return {"Items": [{"LocationType": "FileSystem", "PremiereDate": "2026-06-01T00:00:00Z"}]}
            self.fail("unexpected endpoint: " + path)

        updates = []
        libs = {"追更剧": {"id": "lib-1", "ctype": "tvshows", "folder": "追更剧"}}
        with patch.object(business, "fetch_libs", return_value=libs), \
             patch.object(business, "eget", side_effect=fake_eget), \
             patch.object(business, "task_set", side_effect=lambda *a, **k: updates.append(k)), \
             patch.object(business, "task_is_cancelled", return_value=False):
            result = business.zhuigeng_status_async("tid")

        self.assertEqual(result["items"], [{
            "lib": "追更剧", "name": "示例剧", "id": "series-1", "folder": "示例剧 [tmdbid-1]",
            "tmdb": "1", "status": "Ended", "airing": False, "count": 1, "latest": "2026-06-01"
        }])
        self.assertIn("Status,Path,ProviderIds", [p.get("Fields") for path, p in calls if path == "/Items"])
        self.assertEqual(updates[-1].get("status_text"), "完成")


if __name__ == "__main__":
    unittest.main()
