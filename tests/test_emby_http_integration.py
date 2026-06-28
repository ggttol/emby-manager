import json
import os
import sys
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from lib.config import CFG
import lib.emby as emby


class _FakeEmbyHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"

    def log_message(self, *args):
        pass

    def _read_body(self):
        ln = int(self.headers.get("Content-Length", "0") or "0")
        raw = self.rfile.read(ln) if ln else b""
        try:
            return json.loads(raw.decode("utf-8")) if raw else None
        except Exception:
            return raw.decode("utf-8", "replace")

    def _json(self, code, obj):
        data = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _empty(self, code=204):
        self.send_response(code)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        srv = self.server
        parsed = urlparse(self.path)
        srv.requests.append({"method": "GET", "path": parsed.path, "query": parse_qs(parsed.query)})
        if parsed.path == "/emby/System/Info/Public":
            return self._json(200, {"ServerName": "fake", "Version": "9.9.9"})
        if parsed.path == "/emby/Library/VirtualFolders":
            return self._json(200, [
                {"Name": "Movies", "ItemId": "lib-m", "CollectionType": "movies",
                 "Locations": ["/strm/Movies"]},
                {"Name": "Boxsets", "ItemId": "lib-b", "CollectionType": "boxsets",
                 "Locations": ["/collections"]},
            ])
        if parsed.path == "/emby/Users":
            return self._json(200, srv.users)
        return self._json(200, {"ok": True, "path": parsed.path})

    def do_POST(self):
        srv = self.server
        parsed = urlparse(self.path)
        body = self._read_body()
        srv.requests.append({"method": "POST", "path": parsed.path,
                             "query": parse_qs(parsed.query), "body": body})
        if parsed.path == "/emby/Users/u1/Policy":
            srv.users[0]["Policy"] = body
            return self._empty(204)
        return self._empty(204)

    def do_DELETE(self):
        parsed = urlparse(self.path)
        self.server.requests.append({"method": "DELETE", "path": parsed.path,
                                     "query": parse_qs(parsed.query)})
        return self._empty(204)


class EmbyHttpIntegrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), _FakeEmbyHandler)
        cls.server.requests = []
        cls.server.users = [{"Id": "u1", "Name": "User One", "Policy": {}}]
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base_url = "http://127.0.0.1:%d/emby" % cls.server.server_port
        time.sleep(0.05)

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        self._cfg = dict(CFG)
        CFG["emby_url"] = self.base_url
        CFG["api_key"] = "KEY 1"
        self.server.requests.clear()
        self.server.users = [{"Id": "u1", "Name": "User One", "Policy": {}}]

    def tearDown(self):
        CFG.clear()
        CFG.update(self._cfg)

    def test_url_quotes_path_before_query_string(self):
        url = emby._url("/Items/id?evil=1&x=2#frag/Refresh", {"Mode": "A B"})
        self.assertIn("/Items/id%3Fevil%3D1%26x%3D2%23frag/Refresh?", url)
        parsed = urlparse(url)
        qs = parse_qs(parsed.query)
        self.assertEqual(qs["api_key"], ["KEY 1"])
        self.assertEqual(qs["Mode"], ["A B"])

    def test_eget_epost_edelete_use_expected_method_query_and_body(self):
        info = emby.eget("/System/Info/Public")
        self.assertEqual(info["ServerName"], "fake")
        code = emby.epost("/Items/abc/Refresh", {"Recursive": "true"}, body={"x": 1})
        self.assertEqual(code, 204)
        code = emby.edelete("/Items/abc")
        self.assertEqual(code, 204)
        self.assertEqual([r["method"] for r in self.server.requests], ["GET", "POST", "DELETE"])
        self.assertEqual(self.server.requests[1]["body"], {"x": 1})
        self.assertEqual(self.server.requests[1]["query"]["Recursive"], ["true"])
        for req in self.server.requests:
            self.assertEqual(req["query"]["api_key"], ["KEY 1"])

    def test_fetch_libs_filters_to_strm_virtual_folders(self):
        libs, excluded = emby.fetch_libs_full()
        self.assertEqual(libs, {"Movies": {"id": "lib-m", "ctype": "movies", "folder": "Movies"}})
        self.assertEqual(excluded[0]["name"], "Boxsets")

    def test_update_user_writes_real_policy_fields_and_verifies(self):
        res = emby.update_user("u1", maxsessions=2, bitrate_mbps=8.5, disabled=True)
        self.assertTrue(res["ok"])
        self.assertEqual(res["verify"], {"stream_limit_ok": True, "bitrate_ok": True})
        posts = [r for r in self.server.requests if r["method"] == "POST"]
        self.assertEqual(len(posts), 1)
        policy = posts[0]["body"]
        self.assertEqual(policy["SimultaneousStreamLimit"], 2)
        self.assertEqual(policy["MaxActiveSessions"], 2)
        self.assertEqual(policy["RemoteClientBitrateLimit"], 8500000)
        self.assertTrue(policy["IsDisabled"])


if __name__ == "__main__":
    unittest.main()
