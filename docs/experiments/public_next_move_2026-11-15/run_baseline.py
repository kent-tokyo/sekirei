#!/usr/bin/env python3
"""Run the registered fixed-node baseline on development positions only."""

import argparse
import hashlib
import json
import subprocess
import time
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    corpus = json.loads(args.cases.read_text())
    expected_weight = "634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469"
    if sha256(args.weights) != expected_weight:
        raise ValueError("weights are not the published r6 identity")
    rows = []
    start = time.monotonic()
    stopped = None
    for case in corpus["cases"]:
        if case["split"] != "development":
            continue
        for nodes in (100_000, 1_000_000):
            remaining = 60 - (time.monotonic() - start)
            if remaining < 4:
                stopped = "60s_total_limit"
                break
            command = [str(args.engine), "--sfen", case["initial_sfen"],
                       "--moves", case["initial_usi"], "--expected-sfen", case["sfen"],
                       "--nodes", str(nodes), "--weights", str(args.weights),
                       "--fv-scale", "24", "--nnue-output", "absolute"]
            try:
                result = subprocess.run(command, text=True, capture_output=True,
                                        timeout=min(4, remaining), check=False)
            except subprocess.TimeoutExpired:
                stopped = "per_process_timeout"
                break
            if result.returncode:
                stopped = f"diagnostic_exit_{result.returncode}"
                break
            fields = dict(field.split("=", 1) for field in result.stdout.strip().split("\t"))
            row = {"case_id": case["id"], "nodes_limit": nodes,
                   "bestmove": fields.get("bestmove"), "score_cp": fields.get("score_cp"),
                   "depth": fields.get("depth"), "nodes": fields.get("nodes"),
                   "elapsed_ms": fields.get("elapsed_ms"), "pv_legal": fields.get("pv_legal"),
                   "completed_bound": fields.get("completed_bound"),
                   "completed_iteration_valid": fields.get("completed_iteration_valid"),
                   "aborted": fields.get("aborted"), "abort_reason": fields.get("abort_reason"),
                   "reference_first_move": case["reference_first_move"]}
            rows.append(row)
            if row["pv_legal"] != "true":
                stopped = "invalid_pv"
                break
        if stopped:
            break
    receipt = {"schema": "sekirei.public-next-move-baseline.v1",
               "cases_sha256": sha256(args.cases), "engine_sha256": sha256(args.engine),
               "weights_sha256": expected_weight, "fv_scale": 24, "threads": 1,
               "node_limits": [100_000, 1_000_000], "wall_limit_s": 60,
               "elapsed_s": round(time.monotonic() - start, 3), "stopped": stopped,
               "rows": rows}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"rows": len(rows), "stopped": stopped, "output": str(args.output)}))


if __name__ == "__main__":
    main()
