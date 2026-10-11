#!/usr/bin/env python3
"""Transport tests for the dashboard's loopback HTTP adapter."""

from __future__ import annotations

import http.client
import json
import tempfile
import threading
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path
from types import SimpleNamespace

from gate_dashboard_http import make_handler


class DashboardHttpTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        kifu_dir = Path(self.directory.name)
        (kifu_dir / "game0001.txt").write_text("position startpos\n", encoding="utf-8")
        self.started_payload = None

        def start_run(payload):
            self.started_payload = payload
            return "new-run"

        self.app = SimpleNamespace(
            RUNS={"default": {"kifu_dir": str(kifu_dir)}},
            SHELL_HTML="<!doctype html><title>sekirei</title>",
            POSITION_PRESETS={},
            SECONDS_PER_GAME_ESTIMATE=1,
            get_status_data=lambda run_id: {"run_id": run_id},
            get_runs_data=lambda: {"runs": ["default"]},
            list_weights=lambda: [],
            _count_positions=lambda _path: 0,
            list_pipeline_runs=lambda: [],
            get_pipeline_status=lambda _run_id: None,
            get_history_data=lambda: [],
            get_review_data=lambda kind, ident: {"kind": kind, "ident": ident},
            get_todo_items=lambda: [],
            get_opening_sanity_data=lambda _w1, _w2, _depth: {},
            attach_run=lambda _payload: "attached-run",
            start_run=start_run,
            chat_reply=lambda payload: payload["message"],
        )
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(self.app))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port)

    def tearDown(self) -> None:
        self.connection.close()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.directory.cleanup()

    def request(self, method: str, path: str, body: bytes | None = None):
        headers = {"Content-Type": "application/json"} if body is not None else {}
        self.connection.request(method, path, body=body, headers=headers)
        response = self.connection.getresponse()
        return response, response.read()

    def test_status_is_json_and_never_cached(self) -> None:
        response, body = self.request("GET", "/api/status?run=default")
        self.assertEqual(response.status, 200)
        self.assertEqual(response.getheader("Cache-Control"), "no-store")
        self.assertEqual(json.loads(body), {"run_id": "default"})

    def test_unknown_run_and_kifu_traversal_fail_closed(self) -> None:
        response, _ = self.request("GET", "/api/status?run=missing")
        self.assertEqual(response.status, 404)
        response, _ = self.request("GET", "/kifu/default/../secret")
        self.assertEqual(response.status, 404)

    def test_kifu_and_start_run_routes_preserve_contract(self) -> None:
        response, body = self.request("GET", "/kifu/default/1")
        self.assertEqual(response.status, 200)
        self.assertEqual(body, b"position startpos\n")

        payload = {"candidate": "a", "baseline": "b"}
        response, body = self.request("POST", "/api/runs", json.dumps(payload).encode())
        self.assertEqual(response.status, 200)
        self.assertEqual(json.loads(body), {"run_id": "new-run"})
        self.assertEqual(self.started_payload, payload)


if __name__ == "__main__":
    unittest.main()
