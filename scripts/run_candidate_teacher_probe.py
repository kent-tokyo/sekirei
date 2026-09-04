#!/usr/bin/env python3
"""Run a bounded, same-position search probe for a student and its teacher.

The output deliberately uses the existing ``analysis_record_v1`` shape so the
generic comparison report can be reused.  This is an analysis/cost diagnostic,
not a strength test: it records no game result or Elo claim.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shlex
import subprocess
import time
from pathlib import Path
from typing import Any


INFO = re.compile(
    r"^info depth (?P<depth>\d+) score cp (?P<score>-?\d+) "
    r"nodes (?P<nodes>\d+) nps (?P<nps>\d+) time (?P<time>\d+) .*? pv (?P<pv>\S+)"
)
BEST = re.compile(r"^bestmove (?P<move>\S+)(?: ponder (?P<ponder>\S+))?")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_corpus(path: Path, limit: int) -> list[dict[str, Any]]:
    rows = []
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip():
            continue
        row = json.loads(raw)
        if not isinstance(row.get("sfen"), str) or not row["sfen"]:
            raise ValueError(f"{path}:{line_number}: missing sfen")
        row.setdefault("sample_id", f"probe:{len(rows)}")
        rows.append(row)
        if len(rows) == limit:
            break
    if not rows:
        raise ValueError("corpus is empty")
    return rows


def run_one(engine: Path, weight: Path, row: dict[str, Any], depth: int, timeout: float) -> dict[str, Any]:
    commands = [
        "usi",
        "setoption name Threads value 1",
        "setoption name Parallel value 1",
        "setoption name SpecTopN value 0",
        f"setoption name EvalFile value {weight}",
        "setoption name UseBook value false",
        "isready",
        f"position sfen {row['sfen']}",
        f"go depth {depth}",
    ]
    rendered = " ".join(f"printf '%s\\n' {shlex.quote(command)}" for command in commands)
    shell_command = f"{{ {rendered}; sleep 1; printf '%s\\n' quit; }} | {shlex.quote(str(engine))}"
    started = time.monotonic()
    try:
        completed = subprocess.run(
            ["sh", "-c", shell_command], text=True, capture_output=True, timeout=timeout, check=False
        )
    except subprocess.TimeoutExpired:
        return {"sample_id": row["sample_id"], "status": "timeout", "error_detail": f"timeout>{timeout}s"}
    wall_ms = round((time.monotonic() - started) * 1000, 3)
    lines = [INFO.match(line.strip()) for line in completed.stdout.splitlines()]
    matches = [match for match in lines if match is not None and int(match["depth"]) == depth]
    best = next((BEST.match(line.strip()) for line in completed.stdout.splitlines() if BEST.match(line.strip())), None)
    if completed.returncode != 0 or not matches or best is None:
        return {
            "sample_id": row["sample_id"],
            "status": "engine_error" if completed.returncode else "incomplete",
            "error_detail": completed.stderr[-500:] or "missing depth/bestmove output",
            "wall_time_ms": wall_ms,
        }
    match = matches[-1]
    return {
        "sample_id": row["sample_id"],
        "sfen": row["sfen"],
        "status": "ok",
        "lines": [{
            "depth": depth,
            "score_cp": int(match["score"]),
            "nodes": int(match["nodes"]),
            "nps": int(match["nps"]),
            "time_ms": int(match["time"]),
            "pv": [match["pv"]],
            "multipv": 1,
            "bestmove": best["move"],
        }],
        "wall_time_ms": wall_ms,
        "bestmove": best["move"],
        "ponder": best.group("ponder"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=16)
    parser.add_argument("--depth", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--candidate-output", type=Path, required=True)
    parser.add_argument("--teacher-output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    if args.limit <= 0 or args.depth <= 0 or args.timeout <= 0:
        parser.error("limit, depth, and timeout must be positive")
    corpus = load_corpus(args.corpus, args.limit)
    args.candidate_output.parent.mkdir(parents=True, exist_ok=True)
    args.teacher_output.parent.mkdir(parents=True, exist_ok=True)
    for weight, output in ((args.candidate, args.candidate_output), (args.teacher, args.teacher_output)):
        records = [run_one(args.engine, weight, row, args.depth, args.timeout) for row in corpus]
        output.write_text("\n".join(json.dumps(record, ensure_ascii=False) for record in records) + "\n", encoding="utf-8")
    manifest = {
        "schema_version": 1,
        "corpus_sha256": sha256(args.corpus),
        "positions": len(corpus),
        "depth": args.depth,
        "timeout_s": args.timeout,
        "threads": 1,
        "parallel": 1,
        "spec_top_n": 0,
        "candidate_sha256": sha256(args.candidate),
        "teacher_sha256": sha256(args.teacher),
        "strength_status": "UNMEASURED",
    }
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
