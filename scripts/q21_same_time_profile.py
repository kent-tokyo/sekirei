#!/usr/bin/env python3
"""Capture a fixed same-time T-versus-material search-cost profile.

The input corpus must be a replayable Q21 diagnostic corpus.  This script
records the exact checkpoint digest, elapsed search output, and the number of
static evaluations reported by `sekirei-search-diagnostic`; it does not make a
strength or adoption decision.
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


def parse_profile(text: str) -> dict:
    fields = {}
    for part in text.strip().split("\t"):
        key, separator, value = part.partition("=")
        if separator:
            fields[key] = value
    required = {
        "bestmove",
        "depth",
        "nodes",
        "elapsed_ms",
        "static_evaluations",
        "history_matches_expected",
        "pv_legal",
    }
    missing = sorted(required - fields.keys())
    if missing:
        raise ValueError(f"diagnostic output missing: {', '.join(missing)}")
    if fields["history_matches_expected"] != "true" or fields["pv_legal"] != "true":
        raise ValueError("diagnostic did not preserve the input history or legal PV")
    for key in ("depth", "nodes", "elapsed_ms", "static_evaluations"):
        fields[key] = int(fields[key])
    return fields


def run(binary: Path, entry: dict, time_ms: int, weights: Path | None) -> dict:
    position = entry["position"]
    command = [
        str(binary),
        "--time-ms",
        str(time_ms),
        "--profile-cost",
        "--sfen",
        position["initial_sfen"],
        "--expected-sfen",
        position["sfen"],
    ]
    history = position.get("history_before_usi", [])
    if history:
        command.extend(("--moves", " ".join(history)))
    if weights is not None:
        command.extend(("--weights", str(weights), "--nnue-output", "residual-material"))
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=time_ms / 1000 + 30)
    if completed.returncode:
        raise RuntimeError(f"diagnostic failed ({completed.returncode}): {completed.stderr[-1000:]}")
    return parse_profile(completed.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--time-ms", type=int, required=True)
    parser.add_argument("--forward-ns", type=float, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=8)
    args = parser.parse_args()
    if args.time_ms <= 0 or args.forward_ns <= 0 or args.limit <= 0:
        parser.error("time-ms, forward-ns, and limit must be positive")
    if args.output.exists():
        parser.error("output already exists")
    document = json.loads(args.corpus.read_text(encoding="utf-8"))
    entries = document.get("entries")
    if not isinstance(entries, list) or len(entries) < args.limit:
        parser.error("corpus has fewer entries than --limit")
    if not args.binary.is_file() or not args.weights.is_file():
        parser.error("binary or weights is unavailable")

    rows = []
    for index, entry in enumerate(entries[: args.limit]):
        teacher = run(args.binary, entry, args.time_ms, args.weights)
        material = run(args.binary, entry, args.time_ms, None)
        rows.append({"index": index, "source": entry["source"], "position": entry["position"], "teacher": teacher, "material": material})
    result = {
        "schema": "sekirei.q21-same-time-profile.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "contract": {
            "binary": str(args.binary),
            "weights": str(args.weights),
            "weights_sha256": sha256(args.weights),
            "teacher_output": "residual-material",
            "time_ms": args.time_ms,
            "forward_ns_per_static_evaluation": args.forward_ns,
            "positions": args.limit,
        },
        "rows": rows,
    }
    for row in rows:
        teacher = row["teacher"]
        teacher["estimated_forward_ms"] = round(
            teacher["static_evaluations"] * args.forward_ns / 1_000_000, 3
        )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
