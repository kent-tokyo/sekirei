#!/usr/bin/env python3
"""Diagnose teacher-vs-material losses without making a strength claim.

For a bounded selection of replayable candidate-loss positions, run the same
search diagnostic with the teacher checkpoint and with material-only at two
node budgets.  The output preserves the replay history and raw per-run
transcripts so it can distinguish an evaluator disagreement from a depth-only
change; it deliberately does not choose a new candidate.
"""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def parse_transcript(text):
    values = {}
    for field in text.strip().split("\t"):
        key, separator, value = field.partition("=")
        if separator:
            values[key] = value
    required = {"bestmove", "score_cp", "nodes", "elapsed_ms", "history_matches_expected"}
    missing = sorted(required - values.keys())
    if missing:
        raise ValueError(f"search diagnostic missing fields: {', '.join(missing)}")
    if values["history_matches_expected"] != "true":
        raise ValueError("search diagnostic replay did not match expected SFEN")
    for key in ("score_cp", "nodes", "elapsed_ms", "depth"):
        if key in values:
            values[key] = int(values[key])
    return values


def run(binary, entry, nodes, weights=None, root_move=False):
    position = entry["position"]
    command = [str(binary), "--nodes", str(nodes), "--sfen", position["initial_sfen"]]
    history = position.get("history_before_usi", [])
    if history:
        command += ["--moves", " ".join(history)]
    command += ["--expected-sfen", position["sfen"]]
    if root_move:
        command += ["--root-move", position["actual_move_usi"]]
    evaluator = {"kind": "material", "weights": None, "nnue_output": None}
    if weights is not None:
        command += ["--weights", str(weights), "--nnue-output", "residual-material"]
        evaluator = {
            "kind": "teacher",
            "weights": str(weights),
            "weights_sha256": sha256(weights),
            "nnue_output": "residual-material",
        }
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=180)
    if completed.returncode:
        raise RuntimeError(f"diagnostic failed ({completed.returncode}): {completed.stderr[-1000:]}")
    return {
        "evaluator": evaluator,
        "nodes_budget": nodes,
        "root_move": position["actual_move_usi"] if root_move else None,
        "result": parse_transcript(completed.stdout),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=8)
    parser.add_argument("--shallow-nodes", type=int, default=20_000)
    parser.add_argument("--deep-nodes", type=int, default=100_000)
    args = parser.parse_args()
    document = json.loads(args.corpus.read_text())
    entries = document.get("entries")
    if not isinstance(entries, list) or not entries:
        parser.error("corpus must contain non-empty entries")
    if args.limit <= 0 or args.shallow_nodes <= 0 or args.deep_nodes <= args.shallow_nodes:
        parser.error("limit and node budgets must be positive and deep > shallow")
    selected = entries[: args.limit]
    rows = []
    for index, entry in enumerate(selected):
        runs = [
            run(args.binary, entry, args.shallow_nodes, args.weights),
            run(args.binary, entry, args.deep_nodes, args.weights),
            run(args.binary, entry, args.shallow_nodes),
            run(args.binary, entry, args.deep_nodes),
            run(args.binary, entry, args.deep_nodes, args.weights, root_move=True),
            run(args.binary, entry, args.deep_nodes, root_move=True),
        ]
        rows.append({"index": index, "source": entry["source"], "position": entry["position"], "runs": runs})
    result = {
        "schema": "sekirei.q21-teacher-material-diagnostic.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "input": {"corpus": str(args.corpus), "corpus_sha256": sha256(args.corpus), "selected_entries": len(selected)},
        "contract": {"teacher_output": "residual-material", "shallow_nodes": args.shallow_nodes, "deep_nodes": args.deep_nodes},
        "rows": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(f"wrote {len(rows)} positions to {args.output}")


if __name__ == "__main__":
    main()
