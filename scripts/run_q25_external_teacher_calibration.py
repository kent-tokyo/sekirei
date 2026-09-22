#!/usr/bin/env python3
"""Run the frozen Q25 self-teacher versus external-USI calibration."""

from __future__ import annotations

import argparse
import json
import os
import queue
import re
import subprocess
import threading
import time
from pathlib import Path
from typing import Any

from prepare_q25_external_teacher_calibration import SCHEMA, bind, sha256
from run_core_floodgate_diagnostic import run_position


INFO_SCORE = re.compile(r"\bscore\s+(cp|mate)\s+([^\s]+)")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def atomic_write(path: Path, value: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def parse_score(tokens: list[str]) -> dict[str, Any] | None:
    try:
        index = tokens.index("score")
        kind, raw = tokens[index + 1], tokens[index + 2]
    except (ValueError, IndexError):
        return None
    if kind == "cp":
        try:
            return {"kind": "cp", "value": int(raw)}
        except ValueError:
            return None
    if kind != "mate":
        return None
    if raw in {"+", "-"}:
        return {"kind": "mate", "value": 1 if raw == "+" else -1, "symbolic": raw}
    try:
        return {"kind": "mate", "value": int(raw)}
    except ValueError:
        return None


def parse_info(line: str) -> dict[str, Any] | None:
    tokens = line.split()
    if not tokens or tokens[0] != "info" or "depth" not in tokens or "pv" not in tokens:
        return None
    score = parse_score(tokens)
    if score is None:
        return None
    try:
        depth = int(tokens[tokens.index("depth") + 1])
        multipv = int(tokens[tokens.index("multipv") + 1]) if "multipv" in tokens else 1
        nodes = int(tokens[tokens.index("nodes") + 1]) if "nodes" in tokens else None
        elapsed_ms = int(tokens[tokens.index("time") + 1]) if "time" in tokens else None
        pv = tokens[tokens.index("pv") + 1 :]
    except (ValueError, IndexError):
        return None
    if not pv:
        return None
    return {
        "depth": depth,
        "multipv": multipv,
        "score": score,
        "nodes": nodes,
        "elapsed_ms": elapsed_ms,
        "pv": pv,
        "move": pv[0],
    }


def go_command(depth: int | None, searchmove: str | None, nodes: int | None = None) -> str:
    require((depth is None) != (nodes is None), "specify exactly one external search limit")
    limit = f"depth {depth}" if depth is not None else f"nodes {nodes}"
    if searchmove is None:
        return f"go {limit}"
    # YaneuraOu consumes `searchmoves` as the remainder of the command.
    # Put it after the ordinary limit so `depth` is not silently ignored.
    return f"go {limit} searchmoves {searchmove}"


class LineReader:
    def __init__(self, stream: Any):
        self.lines: queue.Queue[str | None] = queue.Queue()

        def read() -> None:
            for line in stream:
                self.lines.put(line.rstrip("\r\n"))
            self.lines.put(None)

        self.thread = threading.Thread(target=read, daemon=True)
        self.thread.start()

    def until(self, predicate: Any, deadline: float, captured: list[str]) -> str:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("USI response timeout")
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty as error:
                raise TimeoutError("USI response timeout") from error
            if line is None:
                raise RuntimeError("USI engine closed stdout")
            captured.append(line)
            if predicate(line):
                return line


def external_search(
    engine: Path,
    cwd: Path,
    options: dict[str, str],
    sfen: str,
    depth: int | None,
    multipv: int,
    timeout: float,
    searchmove: str | None = None,
    nodes: int | None = None,
) -> dict[str, Any]:
    process = subprocess.Popen(
        [str(engine.resolve())],
        cwd=cwd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    require(process.stdin is not None and process.stdout is not None, "failed to open USI pipes")
    reader = LineReader(process.stdout)
    captured: list[str] = []
    deadline = time.monotonic() + timeout

    def send(command: str) -> None:
        assert process.stdin is not None
        process.stdin.write(command + "\n")
        process.stdin.flush()

    try:
        send("usi")
        reader.until(lambda line: line == "usiok", deadline, captured)
        for name, value in options.items():
            send(f"setoption name {name} value {value}")
        send("isready")
        reader.until(lambda line: line == "readyok", deadline, captured)
        require(any("loading eval file" in line and "nn.bin" in line for line in captured), "missing eval load acknowledgement")
        require(any("Using 1 thread" in line for line in captured), "missing one-thread acknowledgement")
        send("usinewgame")
        send(f"position sfen {sfen}")
        send(go_command(depth, searchmove, nodes))
        best_line = reader.until(lambda line: line.startswith("bestmove "), deadline, captured)
        bestmove = best_line.split()[1]
        parsed = [result for line in captured if (result := parse_info(line)) is not None]
        require(parsed, "external engine returned no parseable score/PV")
        deepest = max(result["depth"] for result in parsed)
        final = [result for result in parsed if result["depth"] == deepest]
        latest: dict[int, dict[str, Any]] = {}
        for result in final:
            latest[result["multipv"]] = result
        lines = [latest[index] for index in sorted(latest) if index <= multipv]
        require(lines and lines[0]["move"] == bestmove, "external bestmove and primary PV differ")
        if searchmove is not None:
            require(bestmove == searchmove, "external forced search returned another move")
        return {
            "completion": "search_completed",
            "bestmove": bestmove,
            "depth": deepest,
            "lines": lines,
            "identity": next((line for line in captured if line.startswith("id name ")), None),
            "eval_load_ack": next((line for line in captured if "loading eval file" in line), None),
            "thread_ack": next((line for line in captured if "Using 1 thread" in line), None),
            "searchmove": searchmove,
            "requested_nodes": nodes,
        }
    except (TimeoutError, RuntimeError, ValueError) as error:
        return {
            "completion": "timeout" if isinstance(error, TimeoutError) else "invalid_output",
            "error": str(error),
            "captured_tail": captured[-40:],
            "searchmove": searchmove,
        }
    finally:
        try:
            if process.stdin is not None:
                process.stdin.write("quit\n")
                process.stdin.flush()
        except (BrokenPipeError, OSError):
            pass
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate()


def self_search(
    engine: Path,
    weights: Path,
    row: dict[str, Any],
    depth: int,
    timeout: float,
    root_candidates: int,
    root_move: str | None = None,
) -> dict[str, Any]:
    return run_position(
        engine,
        row["initial_sfen"],
        1,
        timeout,
        weights,
        root_move=root_move,
        max_depth=depth,
        root_candidates=None if root_move else root_candidates,
        history_moves_usi=row["history_before_usi"],
        expected_sfen=row["sfen"],
        nnue_output="residual-material",
    )


def completed_row(row: dict[str, Any]) -> bool:
    groups = (row.get("self_free", []), row.get("external_free", []))
    if any(not group or any(item.get("completion") != "search_completed" for item in group) for group in groups):
        return False
    self_top = row["self_free"][0].get("bestmove")
    external_top = row["external_free"][0].get("bestmove")
    if self_top == external_top:
        return True
    self_cross = row.get("self_on_external_top", [])
    if not self_cross or any(item.get("completion") != "search_completed" for item in self_cross):
        return False
    external_free_has_self = all(
        any(line.get("move") == self_top for line in result.get("lines", []))
        for result in row["external_free"]
    )
    if external_free_has_self:
        return True
    external_cross = row.get("external_on_self_top", [])
    return bool(external_cross) and all(
        item.get("completion") == "search_completed" for item in external_cross
    )


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    require(prereg.get("schema") == SCHEMA, "unexpected Q25 preregistration schema")
    require(prereg.get("status") == "frozen_before_any_q25_label", "Q25 is not frozen")
    named_paths = {
        "reserve": args.reserve,
        "self_engine": args.self_engine,
        "self_weights": args.self_weights,
        "external_engine": args.external_engine,
        "external_weights": args.external_weights,
    }
    for name, path in named_paths.items():
        require(prereg["inputs"][name]["sha256"] == sha256(path), f"{name} SHA mismatch")
    preregistered_runner_sha = prereg["tools"]["runner"]["sha256"]
    executed_runner_sha = sha256(Path(__file__).resolve())
    reserve = json.loads(args.reserve.read_text(encoding="utf-8"))
    require(reserve.get("schema") == "sekirei.q25-calibration-reserve.v1", "unexpected reserve")
    contract = prereg["measurement_contract"]
    repeats = contract["repeats"]
    self_contract = prereg["self_teacher"]
    external_contract = prereg["external_teacher"]
    require(args.self_timeout > 0 and args.external_timeout > 0, "timeouts must be positive")
    output = {
        "schema": "sekirei.q25-external-teacher-measurements.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration": bind(args.preregistration),
        "tool_provenance": {
            "preregistered_sha256": preregistered_runner_sha,
            "executed_sha256": executed_runner_sha,
            "contract_preserving_fix_after_partial_measurement": (
                preregistered_runner_sha != executed_runner_sha
            ),
            "fix_scope": (
                "place USI searchmoves before depth so YaneuraOu honors the frozen forced move; "
                "rerun only incomplete rows without changing parents, budgets, or thresholds"
            ),
        },
        "rows": [],
    }
    if args.output.is_file():
        prior = json.loads(args.output.read_text(encoding="utf-8"))
        require(
            prior.get("schema") == output["schema"]
            and prior.get("preregistration", {}).get("sha256") == output["preregistration"]["sha256"],
            "existing Q25 output belongs to another contract",
        )
        output = prior
        output["tool_provenance"] = {
            "preregistered_sha256": preregistered_runner_sha,
            "executed_sha256": executed_runner_sha,
            "contract_preserving_fix_after_partial_measurement": (
                preregistered_runner_sha != executed_runner_sha
            ),
            "fix_scope": (
                "place USI searchmoves before depth so YaneuraOu honors the frozen forced move; "
                "rerun only incomplete rows without changing parents, budgets, or thresholds"
            ),
        }
    completed = {row["id"] for row in output["rows"] if completed_row(row)}
    os.environ["RAYON_NUM_THREADS"] = "1"
    for row in reserve["positions"]:
        if row["id"] in completed:
            continue
        self_free = [
            self_search(
                args.self_engine,
                args.self_weights,
                row,
                self_contract["max_depth"],
                args.self_timeout,
                self_contract["root_candidates"],
            )
            for _ in range(repeats)
        ]
        external_free = [
            external_search(
                args.external_engine,
                args.external_engine.parent,
                external_contract["usi_options"],
                row["sfen"],
                external_contract["max_depth"],
                external_contract["multipv"],
                args.external_timeout,
            )
            for _ in range(repeats)
        ]
        self_top = self_free[0].get("bestmove")
        external_top = external_free[0].get("bestmove")
        require(isinstance(self_top, str) and isinstance(external_top, str), f"{row['id']}: missing free bestmove")
        self_on_external = []
        external_on_self = []
        if self_top != external_top:
            self_on_external = [
                self_search(
                    args.self_engine,
                    args.self_weights,
                    row,
                    self_contract["max_depth"],
                    args.self_timeout,
                    self_contract["root_candidates"],
                    external_top,
                )
                for _ in range(repeats)
            ]
            external_on_self = [
                external_search(
                    args.external_engine,
                    args.external_engine.parent,
                    external_contract["usi_options"],
                    row["sfen"],
                    external_contract["max_depth"],
                    1,
                    args.external_timeout,
                    self_top,
                )
                for _ in range(repeats)
            ]
        measurement = {
                "id": row["id"],
                "category": row["category"],
                "sfen": row["sfen"],
                "source": row.get("source"),
                "self_free": self_free,
                "external_free": external_free,
                "self_on_external_top": self_on_external,
                "external_on_self_top": external_on_self,
            }
        output["rows"] = [existing for existing in output["rows"] if existing["id"] != row["id"]]
        output["rows"].append(measurement)
        output["rows"].sort(key=lambda existing: existing["id"])
        atomic_write(args.output, output)
        print(f"Q25 calibration: {len(output['rows'])}/{len(reserve['positions'])}", flush=True)
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--reserve", type=Path, required=True)
    parser.add_argument("--self-engine", type=Path, required=True)
    parser.add_argument("--self-weights", type=Path, required=True)
    parser.add_argument("--external-engine", type=Path, required=True)
    parser.add_argument("--external-weights", type=Path, required=True)
    parser.add_argument("--self-timeout", type=float, default=600.0)
    parser.add_argument("--external-timeout", type=float, default=120.0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"parents": len(result["rows"]), "output": str(args.output)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
