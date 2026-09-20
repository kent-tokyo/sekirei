#!/usr/bin/env python3
"""Separate residual-NNUE evaluation effects from its search cost.

The fixed corpus combines eight Q21 loss positions with eight frozen opening
controls.  Every row is searched with material, loaded NNUE at zero residual
scale, and loaded NNUE at its normal scale.  The zero-scale arm therefore
keeps accumulator and forward-pass cost while preserving material's static
score.  This is diagnostic evidence only; it never selects a strength winner.
"""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def parse(text: str) -> dict:
    result = {}
    for item in text.strip().split("\t"):
        key, separator, value = item.partition("=")
        if separator:
            result[key] = value
    required = {"bestmove", "score_cp", "depth", "nodes", "elapsed_ms", "static_evaluations", "pv_legal", "history_matches_expected"}
    missing = sorted(required - result.keys())
    if missing:
        raise ValueError(f"missing diagnostic fields: {', '.join(missing)}")
    if result["pv_legal"] != "true" or result["history_matches_expected"] != "true":
        raise ValueError("diagnostic did not preserve a legal replay")
    for key in ("score_cp", "depth", "nodes", "elapsed_ms", "static_evaluations"):
        result[key] = int(result[key])
    return result


def run(binary: Path, entry: dict, budget_kind: str, budget: int, weights: Path | None, scale: int | None) -> dict:
    position = entry["position"]
    command = [str(binary), f"--{budget_kind}", str(budget), "--profile-cost", "--sfen", position["initial_sfen"], "--expected-sfen", position["sfen"]]
    history = position.get("history_before_usi", [])
    if history:
        command.extend(("--moves", " ".join(history)))
    if weights is not None:
        command.extend(("--weights", str(weights), "--nnue-output", "residual-material", "--nnue-residual-scale-permille", str(scale)))
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=budget / 1000 + 60 if budget_kind == "time-ms" else 180)
    if completed.returncode:
        raise RuntimeError(f"diagnostic failed ({completed.returncode}): {completed.stderr[-1000:]}")
    return parse(completed.stdout)


def loss_entries(path: Path, limit: int) -> list[dict]:
    document = json.loads(path.read_text(encoding="utf-8"))
    entries = document.get("entries")
    if not isinstance(entries, list) or len(entries) < limit:
        raise ValueError("loss corpus has too few entries")
    return entries[:limit]


def control_entries(path: Path, limit: int) -> list[dict]:
    sfens = [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#")]
    if len(sfens) < limit:
        raise ValueError("control corpus has too few SFENs")
    return [{"source": {"kind": "q19-opening-control", "index": index}, "position": {"initial_sfen": sfen, "sfen": sfen, "history_before_usi": []}} for index, sfen in enumerate(sfens[:limit])]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--loss-corpus", type=Path, required=True)
    parser.add_argument("--control-corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--loss-limit", type=int, default=8)
    parser.add_argument("--control-limit", type=int, default=8)
    parser.add_argument("--nodes", type=int, default=100_000)
    parser.add_argument("--time-ms", type=int, default=1_000)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists")
    if min(args.loss_limit, args.control_limit, args.nodes, args.time_ms) <= 0:
        parser.error("limits and budgets must be positive")
    if not args.binary.is_file() or not args.weights.is_file():
        parser.error("binary or weights is unavailable")

    entries = loss_entries(args.loss_corpus, args.loss_limit) + control_entries(args.control_corpus, args.control_limit)
    arms = (("material", None, None), ("nnue_cost_only", args.weights, 0), ("teacher", args.weights, 1_000))
    rows = []
    for index, entry in enumerate(entries):
        row = {"index": index, "source": entry["source"], "position": entry["position"], "arms": {}}
        for budget_kind, budget in (("nodes", args.nodes), ("time-ms", args.time_ms)):
            row["arms"][budget_kind] = {label: run(args.binary, entry, budget_kind, budget, weights, scale) for label, weights, scale in arms}
        rows.append(row)

    document = {
        "schema": "sekirei.q21-residual-ablation.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "contract": {"binary": str(args.binary), "weights": str(args.weights), "weights_sha256": sha256(args.weights), "loss_corpus": str(args.loss_corpus), "control_corpus": str(args.control_corpus), "nodes": args.nodes, "time_ms": args.time_ms, "arms": {"material": "no weights", "nnue_cost_only": "residual-material scale=0", "teacher": "residual-material scale=1000"}},
        "rows": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
