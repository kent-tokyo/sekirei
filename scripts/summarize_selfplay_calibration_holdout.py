#!/usr/bin/env python3
"""Re-summarize a saved hold-out diagnostic with mate values excluded."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


MATE_ABS_MIN = 899_000


def complete(result: dict) -> bool:
    return result.get("completed_iteration_valid") is True and result.get("completed_bound") == "exact" and result.get("pv_legal") is True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = json.loads(args.input.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.selfplay-calibration-holdout.v1":
        raise SystemExit("unsupported hold-out diagnostic")
    completed = [row for row in document["results"] if all(complete(row[name]) for name in ("material", "baseline", "candidate"))]
    cp_rows = [row for row in completed if all(abs(row[name]["score_cp"]) < MATE_ABS_MIN for name in ("material", "baseline", "candidate"))]
    anchors = [row for row in cp_rows if abs(row["material"]["score_cp"]) >= 1_000]
    baseline_compressed = [row for row in anchors if abs(row["baseline"]["score_cp"]) <= 100]
    candidate_compressed = [row for row in anchors if abs(row["candidate"]["score_cp"]) <= 100]
    base_errors = [abs(row["baseline"]["score_cp"] - row["material"]["score_cp"]) for row in cp_rows]
    candidate_errors = [abs(row["candidate"]["score_cp"] - row["material"]["score_cp"]) for row in cp_rows]
    output = {
        "schema": "sekirei.selfplay-calibration-holdout-summary.v1", "diagnostic_only": True, "strength_claim": False,
        "input": str(args.input), "contract": document["contract"],
        "summary": {"selected": len(document["results"]), "complete": len(completed), "cp_comparable": len(cp_rows), "mate_like_excluded": len(completed) - len(cp_rows), "mate_abs_min": MATE_ABS_MIN, "material_anchors_abs_ge_1000": len(anchors), "baseline_compressed_abs_le_100": len(baseline_compressed), "candidate_compressed_abs_le_100": len(candidate_compressed), "baseline_material_mae": sum(base_errors) / len(base_errors) if base_errors else None, "candidate_material_mae": sum(candidate_errors) / len(candidate_errors) if candidate_errors else None, "signal_pass": len(candidate_compressed) < len(baseline_compressed), "verdict": "REJECTED_FOR_CALIBRATION" if len(candidate_compressed) >= len(baseline_compressed) else "DIAGNOSTIC_SIGNAL_ONLY", "interpretation": "Mate-like search values are excluded from cp calibration. A diagnostic signal is not a playing-strength result or candidate adoption."},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(output["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
