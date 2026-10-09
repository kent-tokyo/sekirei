#!/usr/bin/env python3
"""Run a two-process, 14-game offline CSA tournament rehearsal.

The fixture uses Sekirei's material evaluator and a loopback-only CSA server.
It starts the real ``sekirei-csa`` executable twice, resuming the cumulative
attempt count after the injected process boundary.  The resulting manifest
records binary/config/source identity, every CSA record hash, runtime status
transitions, child PIDs, and proof that both child processes were reaped.
No external evaluator, network service, or credential is used.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import socket
import subprocess
import threading
import time
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "sekirei.denryu-rehearsal.v1"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sha256(document: Any) -> str:
    encoded = json.dumps(
        document, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def git_source_identity() -> dict[str, Any]:
    def git(*args: str) -> bytes:
        return subprocess.check_output(["git", *args], cwd=ROOT, stderr=subprocess.DEVNULL)

    try:
        revision = git("rev-parse", "HEAD").decode().strip()
        status = git("status", "--porcelain=v1")
        diff = git("diff", "--binary", "HEAD")
    except (OSError, subprocess.CalledProcessError):
        return {"revision": "unknown", "dirty": None, "sha256": None}
    identity = revision.encode() + b"\n" + status + b"\n" + diff
    return {
        "revision": revision,
        "dirty": bool(status),
        "sha256": hashlib.sha256(identity).hexdigest(),
    }


def read_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} is not a JSON object")
    return value


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
        if not isinstance(row, dict):
            raise ValueError(f"{path}:{line_number}: row is not an object")
        rows.append(row)
    return rows


def process_exists(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


class OfflineCsaServer:
    def __init__(self, games_per_phase: tuple[int, int]):
        self.games_per_phase = games_per_phase
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(2)
        self.listener.settimeout(15.0)
        self.port = self.listener.getsockname()[1]
        self.error: BaseException | None = None
        self.thread = threading.Thread(target=self._serve, name="denryu-rehearsal-server")

    def start(self) -> None:
        self.thread.start()

    def join(self) -> None:
        self.thread.join(timeout=20.0)
        if self.thread.is_alive():
            raise TimeoutError("offline CSA server did not stop")
        if self.error is not None:
            raise RuntimeError("offline CSA server failed") from self.error

    @staticmethod
    def _readline(stream: Any) -> str:
        line = stream.readline()
        if not line:
            raise EOFError("CSA client closed the connection unexpectedly")
        return line.decode("utf-8").rstrip("\r\n")

    def _serve_phase(self, connection: socket.socket, first_game: int, count: int) -> None:
        connection.settimeout(10.0)
        with connection, connection.makefile("rwb") as stream:
            if not self._readline(stream).startswith("LOGIN rehearsal "):
                raise AssertionError("unexpected LOGIN command")
            stream.write(b"LOGIN: rehearsal OK\n")
            stream.flush()
            for index in range(first_game, first_game + count):
                if not self._readline(stream).startswith("%%GAME denryu-rehearsal "):
                    raise AssertionError("unexpected game request")
                black = index % 2 == 1
                turn = "+" if black else "-"
                total = 180 if black else 600
                game_id = f"denryu-rehearsal-{index:02d}"
                summary = (
                    "BEGIN Game_Summary\n"
                    f"Game_ID:{game_id}\n"
                    "Name+:sekirei-black\n"
                    "Name-:sekirei-white\n"
                    f"Your_Turn:{turn}\n"
                    "BEGIN Time\n"
                    "Time_Unit:1sec\n"
                    f"Total_Time:{total}\n"
                    "Increment:2\n"
                    "END Time\n"
                    "END Game_Summary\n"
                    "BEGIN Position\nPI\nEND Position\n"
                    f"START:{game_id}\n"
                )
                stream.write(summary.encode("utf-8"))
                stream.flush()
                if self._readline(stream) != f"AGREE:{game_id}":
                    raise AssertionError("unexpected AGREE command")
                if not black:
                    stream.write(b"+7776FU,T0\n")
                    stream.flush()
                move = self._readline(stream)
                if not move.startswith(turn):
                    raise AssertionError(f"unexpected client move: {move}")
                stream.write(f"{move},T0\n#WIN\n".encode("utf-8"))
                stream.flush()

    def _serve(self) -> None:
        try:
            first_game = 1
            for count in self.games_per_phase:
                connection, _ = self.listener.accept()
                self._serve_phase(connection, first_game, count)
                first_game += count
        except BaseException as error:  # surfaced by join()
            self.error = error
        finally:
            self.listener.close()


def run_phase(
    binary: Path,
    output: Path,
    port: int,
    phase: int,
    completed: int,
    limit: int,
    journal: Path,
) -> dict[str, Any]:
    phase_root = output / f"phase-{phase}"
    phase_root.mkdir(parents=True, exist_ok=False)
    command = [
        str(binary),
        "--server", "127.0.0.1",
        "--port", str(port),
        "--user", "rehearsal",
        "--password", "loopback-only",
        "--game", "denryu-rehearsal",
        "--eval", "material",
        "--depth", "1",
        "--hash", "1",
        "--record-dir", str(phase_root / "records"),
        "--run-manifest", str(phase_root / "run-manifest.json"),
        "--status-file", str(phase_root / "status.json"),
        "--status-journal", str(journal),
        "--loop",
        "--max-games", str(limit),
        "--completed-attempts", str(completed),
    ]
    started_ms = time.time_ns() // 1_000_000
    with (phase_root / "stdout.log").open("wb") as stdout, (
        phase_root / "stderr.log"
    ).open("wb") as stderr:
        process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
        pid = process.pid
        try:
            return_code = process.wait(timeout=30.0)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5.0)
            raise RuntimeError(f"phase {phase} timed out")
    ended_ms = time.time_ns() // 1_000_000
    if return_code != 0:
        raise RuntimeError(
            f"phase {phase} exited {return_code}; see {phase_root / 'stderr.log'}"
        )
    if process_exists(pid):
        raise RuntimeError(f"phase {phase} child PID {pid} remains after wait")
    status = read_json(phase_root / "status.json")
    manifest = read_json(phase_root / "run-manifest.json")
    expected_games = limit - completed
    records = sorted((phase_root / "records").glob("*.csa"))
    if status.get("completed_attempts") != limit:
        raise ValueError(f"phase {phase} status did not reach {limit}")
    if status.get("terminal_stop_reason") != "max_games_reached":
        raise ValueError(f"phase {phase} did not stop at the game ceiling")
    if manifest.get("completed_attempts") != limit:
        raise ValueError(f"phase {phase} manifest did not reach {limit}")
    if len(manifest.get("games", [])) != expected_games:
        raise ValueError(f"phase {phase} manifest game count is not {expected_games}")
    if len(records) != expected_games:
        raise ValueError(f"phase {phase} record count is not {expected_games}")
    return {
        "phase": phase,
        "pid": pid,
        "started_ms": started_ms,
        "ended_ms": ended_ms,
        "return_code": return_code,
        "initial_completed_attempts": completed,
        "final_completed_attempts": limit,
        "process_reaped": True,
        "manifest": str((phase_root / "run-manifest.json").relative_to(output)),
        "status": str((phase_root / "status.json").relative_to(output)),
        "records": [
            {
                "path": str(path.relative_to(output)),
                "bytes": path.stat().st_size,
                "sha256": sha256_file(path),
            }
            for path in records
        ],
    }


def validate_journal(journal: Path, phases: list[dict[str, Any]], final_limit: int) -> int:
    rows = read_jsonl(journal)
    if not rows:
        raise ValueError("runtime status journal is empty")
    expected_pids = {phase["pid"] for phase in phases}
    observed_pids = {row.get("pid") for row in rows}
    if expected_pids != observed_pids:
        raise ValueError(f"journal PIDs differ: expected {expected_pids}, got {observed_pids}")
    attempts = [row.get("completed_attempts") for row in rows]
    if not all(isinstance(value, int) for value in attempts):
        raise ValueError("journal has a non-integer completed_attempts value")
    if attempts != sorted(attempts):
        raise ValueError("journal completed_attempts values are not monotonic")
    if attempts[0] != 0 or attempts[-1] != final_limit:
        raise ValueError("journal does not cover the complete attempt range")
    stopped = [
        row for row in rows
        if row.get("state") == "stopped" and row.get("terminal_stop_reason") == "max_games_reached"
    ]
    if [row.get("completed_attempts") for row in stopped] != [
        phase["final_completed_attempts"] for phase in phases
    ]:
        raise ValueError("journal does not contain both terminal process boundaries")
    return len(rows)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary", type=Path, default=ROOT / "target/release/sekirei-csa",
        help="sekirei-csa executable (default: target/release/sekirei-csa)",
    )
    parser.add_argument("--output", type=Path, required=True, help="new evidence directory")
    parser.add_argument("--games", type=int, default=14)
    parser.add_argument("--restart-after", type=int, default=7)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    binary = args.binary.resolve()
    output = args.output.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SystemExit(f"executable not found: {binary}")
    if args.games < 2 or not 0 < args.restart_after < args.games:
        raise SystemExit("--restart-after must be between 1 and --games - 1")
    output.mkdir(parents=True, exist_ok=False)
    journal = output / "status-transitions.jsonl"
    phase_counts = (args.restart_after, args.games - args.restart_after)
    server = OfflineCsaServer(phase_counts)
    server.start()
    phases: list[dict[str, Any]] = []
    try:
        phases.append(run_phase(binary, output, server.port, 1, 0, args.restart_after, journal))
        phases.append(
            run_phase(
                binary, output, server.port, 2,
                args.restart_after, args.games, journal,
            )
        )
        server.join()
    finally:
        if server.thread.is_alive():
            server.listener.close()
            server.thread.join(timeout=1.0)
    record_count = sum(len(phase["records"]) for phase in phases)
    if record_count != args.games:
        raise ValueError(f"expected {args.games} records, found {record_count}")
    journal_rows = validate_journal(journal, phases, args.games)
    configuration = {
        "evaluation": "material",
        "hash_mb": 1,
        "max_depth": 1,
        "request_game_id": "denryu-rehearsal",
        "games": args.games,
        "restart_after": args.restart_after,
        "time_control": {
            "black_total_seconds": 180,
            "white_total_seconds": 600,
            "increment_seconds": 2,
        },
    }
    document = {
        "schema": SCHEMA,
        "status": "passed",
        "created_at_ms": time.time_ns() // 1_000_000,
        "binary": {
            "path": str(binary),
            "bytes": binary.stat().st_size,
            "sha256": sha256_file(binary),
        },
        "model": {"active": False, "sha256": None},
        "configuration": configuration,
        "configuration_sha256": canonical_sha256(configuration),
        "source": git_source_identity(),
        "phases": phases,
        "record_count": record_count,
        "status_transition_count": journal_rows,
        "all_children_reaped": all(phase["process_reaped"] for phase in phases),
    }
    manifest = output / "rehearsal-manifest.json"
    manifest.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"PASS: {record_count} games, 2 processes, evidence={manifest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
