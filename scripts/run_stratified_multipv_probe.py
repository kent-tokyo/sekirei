#!/usr/bin/env python3
"""Run a deterministic, stratified MultiPV table probe for three evaluators."""

from __future__ import annotations

import argparse
import json
import re
import shlex
import subprocess
import time
from pathlib import Path

from classify_depth3_patterns import features, load as load_records
from run_candidate_teacher_probe import render_usi_commands, sha256


INFO = re.compile(
    r"^info multipv (?P<multipv>\d+) depth (?P<depth>\d+) score cp (?P<score>-?\d+) "
    r"nodes (?P<nodes>\d+) nps (?P<nps>\d+) time (?P<time>\d+) .*? pv (?P<pv>\S+)"
)
BEST = re.compile(r"^bestmove (?P<move>\S+)")


def choose_rows(corpus: dict[str, dict], candidate: dict[str, dict], teacher: dict[str, dict], material: dict[str, dict], per_group: int) -> list[dict]:
    rows = []
    for sample_id in sorted(candidate.keys() & teacher.keys() & material.keys()):
        c, t, m = candidate[sample_id], teacher[sample_id], material[sample_id]
        if not all(row.get("status") == "ok" for row in (c, t, m)):
            continue
        if c["bestmove"] == t["bestmove"]:
            continue
        cs, ts = int(c["lines"][0]["score_cp"]), int(t["lines"][0]["score_cp"])
        move_bucket = "candidate_material" if c["bestmove"] == m["bestmove"] else "teacher_material" if t["bestmove"] == m["bestmove"] else "all_distinct"
        sign_bucket = "opposite_nonzero" if (cs > 0) != (ts > 0) and cs and ts else "same_sign_or_zero"
        rows.append({"sample_id": sample_id, **features(corpus[sample_id]["sfen"]), "move_bucket": move_bucket, "sign_bucket": sign_bucket})
    selected = []
    for phase in ("early", "middle", "late"):
        for bucket in ("all_distinct", "candidate_material", "teacher_material"):
            group = [row for row in rows if row["phase"] == phase and row["move_bucket"] == bucket]
            selected.extend(group[:per_group])
    return selected


def run_one(engine: Path, weight: Path | None, row: dict, depth: int, multipv: int, timeout: float) -> dict:
    commands = [
        "usi",
        "setoption name Threads value 1",
        "setoption name Parallel value 1",
        "setoption name SpecTopN value 0",
        "setoption name UseBook value false",
        f"setoption name MultiPV value {multipv}",
        "isready",
        f"position sfen {row['sfen']}",
        f"go depth {depth}",
    ]
    if weight is not None:
        commands.insert(4, f"setoption name EvalFile value {weight}")
    shell_command = f"{{ {render_usi_commands(commands)}; sleep 1; printf '%s\\n' quit; }} | {shlex.quote(str(engine))}"
    started = time.monotonic()
    try:
        completed = subprocess.run(["sh", "-c", shell_command], text=True, capture_output=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired:
        return {"status": "timeout", "error_detail": f"timeout>{timeout}s"}
    wall_ms = round((time.monotonic() - started) * 1000, 3)
    lines = []
    for raw in completed.stdout.splitlines():
        match = INFO.match(raw.strip())
        if match and int(match["depth"]) == depth:
            lines.append({"multipv": int(match["multipv"]), "score_cp": int(match["score"]), "nodes": int(match["nodes"]), "nps": int(match["nps"]), "time_ms": int(match["time"]), "move": match["pv"]})
    best = next((match["move"] for raw in completed.stdout.splitlines() if (match := BEST.match(raw.strip()))), None)
    if completed.returncode != 0 or not lines or best is None:
        return {"status": "incomplete", "error_detail": completed.stderr[-500:] or "missing MultiPV/bestmove output", "wall_time_ms": wall_ms}
    return {"status": "ok", "bestmove": best, "lines": sorted(lines, key=lambda line: line["multipv"]), "wall_time_ms": wall_ms}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--material", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--candidate-depth3", type=Path, required=True)
    parser.add_argument("--teacher-depth3", type=Path, required=True)
    parser.add_argument("--material-depth3", type=Path, required=True)
    parser.add_argument("--depth", type=int, default=3)
    parser.add_argument("--multipv", type=int, default=5)
    parser.add_argument("--per-group", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if min(args.depth, args.multipv, args.per_group) <= 0 or args.timeout <= 0:
        parser.error("depth, multipv, per-group, and timeout must be positive")
    corpus, candidate, teacher, material = (load_records(path) for path in (args.corpus, args.candidate_depth3, args.teacher_depth3, args.material_depth3))
    selected = choose_rows(corpus, candidate, teacher, material, args.per_group)
    evaluators = {"candidate": args.candidate, "teacher": args.teacher, "material": None}
    report = {"schema_version": 1, "diagnostic_only": True, "depth": args.depth, "multipv": args.multipv, "per_group": args.per_group, "corpus_sha256": sha256(args.corpus), "positions": []}
    for row in selected:
        result = {"sample_id": row["sample_id"], "sfen_features": {key: row[key] for key in ("phase", "side", "move_bucket", "sign_bucket")}, "evaluators": {}}
        for name, weight in evaluators.items():
            result["evaluators"][name] = run_one(args.engine, weight, corpus[row["sample_id"]], args.depth, args.multipv, args.timeout)
        report["positions"].append(result)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"schema_version": 1, "diagnostic_only": True, "positions": len(selected), "output": str(args.output)}, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
