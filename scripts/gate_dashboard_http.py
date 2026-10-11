"""HTTP transport for :mod:`gate_dashboard`.

The dashboard module owns deterministic review and run-state policy.  This
module only maps HTTP requests to that application object, keeping transport
errors and browser caching concerns out of the review logic.
"""

from __future__ import annotations

import json
import os
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import ModuleType


def make_handler(app: ModuleType) -> type[BaseHTTPRequestHandler]:
    """Build a request handler bound to one loaded dashboard application."""

    class DashboardHandler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):
            pass

        def _send_json(self, obj, status=200):
            body = json.dumps(obj).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def _send_not_found(self, message=b"not found"):
            self.send_response(404)
            self.end_headers()
            self.wfile.write(message)

        def _send_kifu(self, run_id, n_str):
            run = app.RUNS.get(run_id)
            kifu_dir = run["kifu_dir"] if run else None
            if not kifu_dir or not n_str.isdigit():
                self._send_not_found()
                return
            path = os.path.join(kifu_dir, f"game{int(n_str):04d}.txt")
            try:
                with open(path, "rb") as source:
                    body = source.read()
            except OSError:
                self._send_not_found(b"kifu not found")
                return
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            parsed = urllib.parse.urlparse(self.path)
            path = parsed.path
            query = urllib.parse.parse_qs(parsed.query)
            if path == "/api/status":
                run_id = query.get("run", ["default"])[0]
                if run_id not in app.RUNS:
                    self._send_json({"error": "unknown run"}, status=404)
                    return
                self._send_json(app.get_status_data(run_id))
                return
            if path == "/api/runs":
                self._send_json(app.get_runs_data())
                return
            if path == "/api/weights":
                presets = {
                    key: {
                        "file": os.path.basename(value),
                        "positions": app._count_positions(value),
                    }
                    for key, value in app.POSITION_PRESETS.items()
                }
                self._send_json(
                    {
                        "weights": app.list_weights(),
                        "position_presets": presets,
                        "seconds_per_game_estimate": app.SECONDS_PER_GAME_ESTIMATE,
                    }
                )
                return
            if path == "/api/pipeline_runs":
                self._send_json(app.list_pipeline_runs())
                return
            if path == "/api/pipeline":
                run_id = query.get("run", [""])[0]
                status = app.get_pipeline_status(run_id) if run_id else None
                if status is None:
                    self._send_json({"error": "unknown pipeline run"}, status=404)
                    return
                self._send_json(status)
                return
            if path == "/api/history":
                self._send_json(app.get_history_data())
                return
            if path == "/api/review":
                kind = query.get("kind", [""])[0]
                ident = query.get("run", query.get("file", [""]))[0]
                try:
                    self._send_json(app.get_review_data(kind, ident))
                except ValueError as error:
                    self._send_json({"error": str(error)}, status=400)
                return
            if path == "/api/todo":
                self._send_json(app.get_todo_items())
                return
            if path == "/api/opening_sanity":
                w1 = query.get("w1", [""])[0]
                w2 = query.get("w2", [""])[0]
                depth = query.get("depth", ["6"])[0]
                try:
                    self._send_json(app.get_opening_sanity_data(w1, w2, depth))
                except (ValueError, RuntimeError, OSError, json.JSONDecodeError) as error:
                    self._send_json({"error": str(error)}, status=400)
                return
            if path.startswith("/kifu/"):
                parts = path[len("/kifu/") :].split("/", 1)
                if len(parts) == 2:
                    self._send_kifu(parts[0], parts[1])
                else:
                    self._send_not_found()
                return
            if path in ("/", "/status", "/history", "/strength"):
                body = app.SHELL_HTML.encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            self._send_not_found()

        def do_POST(self):
            parsed = urllib.parse.urlparse(self.path)
            if parsed.path in ("/api/runs", "/api/runs/attach"):
                length = int(self.headers.get("Content-Length", 0) or 0)
                body = self.rfile.read(length) if length else b"{}"
                try:
                    payload = json.loads(body)
                    run_id = (
                        app.attach_run(payload)
                        if parsed.path == "/api/runs/attach"
                        else app.start_run(payload)
                    )
                    self._send_json({"run_id": run_id})
                except (ValueError, RuntimeError, OSError, json.JSONDecodeError) as error:
                    self._send_json({"error": str(error)}, status=400)
                return
            if parsed.path == "/api/chat":
                length = int(self.headers.get("Content-Length", 0) or 0)
                body = self.rfile.read(length) if length else b"{}"
                try:
                    payload = json.loads(body)
                    self._send_json({"reply": app.chat_reply(payload)})
                except (ValueError, RuntimeError, OSError, json.JSONDecodeError) as error:
                    self._send_json({"error": str(error)}, status=400)
                return
            self._send_not_found()

    return DashboardHandler


def serve_dashboard(app: ModuleType, port: int) -> None:
    """Serve one dashboard application on loopback until interrupted."""
    server = ThreadingHTTPServer(("127.0.0.1", port), make_handler(app))
    print(f"http://127.0.0.1:{port}")
    server.serve_forever()

