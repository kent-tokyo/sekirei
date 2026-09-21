#!/usr/bin/env python3
"""Run the preregistered Q21j content-versus-cost failure audit.

The fixed-node cells isolate evaluator content.  The candidate at residual
scale zero keeps its NNUE accumulator/forward cost while producing material
static scores.  The fixed-time cells then measure the practical search-budget
loss from that cost.  A forced-root fixed-node cell measures regret of the
move actually played in the Q21i match.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path


ARMS = ("material", "candidate-cost-only", "candidate", "teacher")
ROTATIONS = (
    ARMS,
    ("teacher", "material", "candidate-cost-only", "candidate"),
    ("candidate", "teacher", "material", "candidate-cost-only"),
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git_metadata(root: Path) -> dict:
    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root, text=True, capture_output=True, check=True
    ).stdout.strip()
    dirty = bool(subprocess.run(
        ["git", "status", "--porcelain"], cwd=root, text=True, capture_output=True, check=True
    ).stdout.strip())
    return {"revision": revision, "dirty": dirty}


def parse_profile(text: str) -> dict:
    fields = {}
    for part in text.strip().split("\t"):
        key, separator, value = part.partition("=")
        if separator:
            fields[key] = value
    required = {
        "bestmove", "depth", "score_cp", "nodes", "elapsed_ms", "bound",
        "completed_bound", "completed_iteration_valid", "aborted", "abort_reason",
        "static_evaluations", "pv_legal", "pv_replay_preserves_input",
        "history_final_hash", "history_matches_expected",
    }
    missing = sorted(required - fields.keys())
    if missing:
        raise ValueError(f"diagnostic output missing: {', '.join(missing)}")
    for key in ("depth", "score_cp", "nodes", "elapsed_ms", "static_evaluations"):
        fields[key] = int(fields[key])
    for key in ("completed_iteration_valid", "aborted", "pv_legal",
                "pv_replay_preserves_input", "history_matches_expected"):
        if fields[key] not in {"true", "false"}:
            raise ValueError(f"invalid boolean {key}={fields[key]!r}")
        fields[key] = fields[key] == "true"
    if not all(fields[key] for key in ("pv_legal", "pv_replay_preserves_input", "history_matches_expected")):
        raise ValueError("diagnostic failed history/PV validation")
    return fields


def arm_options(arm: str, candidate: Path, teacher: Path) -> tuple[Path | None, int | None]:
    if arm == "material":
        return None, None
    if arm == "teacher":
        return teacher, 1_000
    return candidate, 0 if arm == "candidate-cost-only" else 1_000


def measurement_plan(position_ids: list[str]) -> list[dict]:
    plan = []
    for repetition, order in enumerate(ROTATIONS):
        for position_id in position_ids:
            for arm in order:
                plan.append({"cell": "fixed_nodes_free", "repetition": repetition,
                             "position_id": position_id, "arm": arm})
    for position_id in position_ids:
        for arm in ARMS:
            plan.append({"cell": "fixed_nodes_actual", "repetition": 0,
                         "position_id": position_id, "arm": arm})
    for repetition, order in enumerate(ROTATIONS):
        for position_id in position_ids:
            for arm in order:
                plan.append({"cell": "fixed_time_free", "repetition": repetition,
                             "position_id": position_id, "arm": arm})
    return plan


def key(row: dict) -> tuple[str, int, str, str]:
    return row["cell"], row["repetition"], row["position_id"], row["arm"]


def run_search(binary: Path, candidate: Path, teacher: Path, position: dict,
               cell: str, amount: int, arm: str) -> dict:
    command = [str(binary), "--profile-cost", "--sfen", position["initial_sfen"]]
    command.extend(("--moves", " ".join(position["history_before_usi"])))
    command.extend(("--expected-sfen", position["sfen"]))
    if cell.startswith("fixed_nodes"):
        command.extend(("--nodes", str(amount)))
        timeout = 180.0
    else:
        command.extend(("--time-ms", str(amount)))
        timeout = amount / 1000 + 30.0
    if cell == "fixed_nodes_actual":
        command.extend(("--root-move", position["actual_move_usi"]))
    weights, scale = arm_options(arm, candidate, teacher)
    if weights is not None:
        command.extend(("--weights", str(weights), "--nnue-output", "residual-material",
                        "--nnue-residual-scale-permille", str(scale)))
    env = dict(os.environ)
    env["RAYON_NUM_THREADS"] = "1"
    completed = subprocess.run(command, text=True, capture_output=True, check=False,
                               timeout=timeout, env=env)
    if completed.returncode:
        raise RuntimeError(f"diagnostic failed ({completed.returncode}): {completed.stderr[-2000:]}")
    return parse_profile(completed.stdout)


def load_existing(path: Path) -> dict[tuple[str, int, str, str], dict]:
    if not path.is_file():
        return {}
    rows = {}
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        row_key = key(row)
        if row_key in rows:
            raise ValueError(f"duplicate result key at line {line_number}: {row_key}")
        rows[row_key] = row
    return rows


def append_result(path: Path, row: dict) -> None:
    with path.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(row, sort_keys=True) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=100_000)
    parser.add_argument("--time-ms", type=int, default=1_000)
    args = parser.parse_args()
    if args.nodes <= 0 or args.time_ms <= 0:
        parser.error("nodes and time-ms must be positive")
    for path in (args.corpus, args.binary, args.candidate, args.teacher):
        if not path.is_file():
            parser.error(f"missing input: {path}")
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    positions = corpus.get("positions", [])
    if corpus.get("schema") != "sekirei.q21j-failure-audit-corpus.v1" or len(positions) != 32:
        parser.error("corpus must contain the frozen 32-game Q21j selection")
    by_id = {position["id"]: position for position in positions}
    if len(by_id) != 32:
        parser.error("Q21j position IDs are not unique")

    root = Path(__file__).resolve().parent.parent
    preregistration = {
        "schema": "sekirei.q21j-failure-audit-preregistration.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "question": "separate candidate evaluation content from residual-NNUE compute cost on all 32 Q21i games",
        "contract": {
            "fixed_nodes": args.nodes, "fixed_time_ms": args.time_ms,
            "free_repetitions": 3, "forced_actual_repetitions": 1,
            "rayon_threads": 1, "arms": {
                "material": "no weights",
                "candidate-cost-only": "candidate weights, residual-material scale=0",
                "candidate": "candidate weights, residual-material scale=1000",
                "teacher": "teacher weights, residual-material scale=1000",
            },
            "cost_identity_requirement": "material and candidate-cost-only fixed-node score/bestmove match >=95%",
            "cost_pressure_signal": "median fixed-time candidate-cost-only/material node ratio <=0.75",
            "content_signal": "fixed-node candidate/material bestmove disagreement or >=300cp score difference; report actual-move regret separately",
            "attribution_boundary": "classify cost and content signals separately; do not infer a causal percentage of match losses",
        },
        "inputs": {
            "corpus": {"path": str(args.corpus), "sha256": sha256(args.corpus)},
            "binary": {"path": str(args.binary), "sha256": sha256(args.binary)},
            "candidate": {"path": str(args.candidate), "sha256": sha256(args.candidate)},
            "teacher": {"path": str(args.teacher), "sha256": sha256(args.teacher)},
            "runner_source": {"path": str(Path(__file__).resolve()), "sha256": sha256(Path(__file__).resolve())},
        },
        "position_ids": list(by_id),
        "git": git_metadata(root),
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    prereg_path = args.output_dir / "preregistration.json"
    encoded_prereg = json.dumps(preregistration, indent=2, sort_keys=True) + "\n"
    if prereg_path.exists() and prereg_path.read_text(encoding="utf-8") != encoded_prereg:
        parser.error("existing preregistration differs; use a fresh output directory")
    prereg_path.write_text(encoded_prereg, encoding="utf-8")

    results_path = args.output_dir / "measurements.jsonl"
    try:
        existing = load_existing(results_path)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    plan = measurement_plan(list(by_id))
    for index, item in enumerate(plan, 1):
        item_key = key(item)
        if item_key in existing:
            continue
        amount = args.time_ms if item["cell"] == "fixed_time_free" else args.nodes
        result = run_search(args.binary, args.candidate, args.teacher,
                            by_id[item["position_id"]]["position"], item["cell"], amount, item["arm"])
        row = {**item, "amount": amount,
               "game_num": by_id[item["position_id"]]["game_num"],
               "game_result": by_id[item["position_id"]]["game_result"],
               "actual_move_usi": by_id[item["position_id"]]["position"]["actual_move_usi"],
               "result": result}
        append_result(results_path, row)
        existing[item_key] = row
        if index % 32 == 0 or index == len(plan):
            print(f"progress {len(existing)}/{len(plan)}", flush=True)
    manifest = {
        "schema": "sekirei.q21j-failure-audit-measurements.v1",
        "status": "complete",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration_sha256": sha256(prereg_path),
        "measurements": len(existing),
        "expected_measurements": len(plan),
        "measurements_path": str(results_path),
        "measurements_sha256": sha256(results_path),
    }
    (args.output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
