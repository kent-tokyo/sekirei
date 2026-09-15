#!/usr/bin/env python3
"""Classify a completed gate as formal evidence or diagnostic-only evidence.

The audit never rewrites a historical plan or invents missing execution
metadata.  It records why an older run may still be useful for diagnosis while
being ineligible for a release-strength claim.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


REQUIRED_SPRT = {
    "elo0": 0,
    "elo1": 20,
    "alpha": 0.05,
    "beta": 0.05,
    "variant": "trinomial",
    "paired_by_id": True,
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(path: Path) -> dict[str, object] | None:
    if not path.is_file():
        return None
    return {"path": str(path), "sha256": sha256(path), "bytes": path.stat().st_size}


def audit(run_dir: Path) -> dict[str, object]:
    plan_path = run_dir / "plan.json"
    records_path = run_dir / "combined.jsonl"
    plan = json.loads(plan_path.read_text(encoding="utf-8")) if plan_path.is_file() else None
    records = (
        [json.loads(line) for line in records_path.read_text(encoding="utf-8").splitlines() if line.strip()]
        if records_path.is_file()
        else []
    )
    reasons: list[str] = []
    protocol = plan.get("protocol") if isinstance(plan, dict) else None
    if not isinstance(plan, dict):
        reasons.append("missing_plan")
    if not isinstance(protocol, dict):
        reasons.append("missing_protocol")
        protocol = {}
    if protocol.get("games_per_position") != 2:
        reasons.append("games_per_position_not_two")
    if protocol.get("positions") != 200:
        reasons.append("positions_not_two_hundred")
    if protocol.get("max_games") != 400:
        reasons.append("max_games_not_four_hundred")
    if protocol.get("sprt") != REQUIRED_SPRT:
        reasons.append("sprt_not_frozen_paired_trinomial")
    evaluation = plan.get("evaluation") if isinstance(plan, dict) else None
    if not isinstance(evaluation, dict):
        reasons.append("missing_nnue_evaluation_binding")
    else:
        for arm in ("candidate", "baseline"):
            binding = evaluation.get(arm)
            if not isinstance(binding, dict) or binding.get("mode") not in {"absolute", "residual-material"}:
                reasons.append(f"invalid_{arm}_nnue_output_mode")

    execution = artifact(run_dir / "execution.json")
    state = artifact(run_dir / "state.json")
    preflight = artifact(run_dir / "preflight.json")
    if execution is None:
        reasons.append("missing_execution_manifest")
    if state is None:
        reasons.append("missing_durable_state")
    if preflight is None:
        reasons.append("missing_preflight")
    if not records_path.is_file():
        reasons.append("missing_combined_jsonl")

    pair_counts = Counter(record.get("id") for record in records)
    invalid_pair_ids = sum(1 for pair_id in pair_counts if not isinstance(pair_id, str) or not pair_id)
    incomplete_pairs = sum(1 for count in pair_counts.values() if count != 2)
    if invalid_pair_ids:
        reasons.append("records_missing_pair_id")
    if incomplete_pairs:
        reasons.append("records_have_incomplete_pairs")
    invalid_results = sum(
        1 for record in records if record.get("result") not in {"candidate_win", "baseline_win", "draw"}
    )
    if invalid_results:
        reasons.append("records_have_invalid_results")

    outcomes = Counter(record.get("result") for record in records)
    formal = not reasons
    return {
        "schema": "sekirei.strength-gate-audit.v1",
        "run_directory": str(run_dir),
        "formal_eligibility": formal,
        "classification": "formal_candidate" if formal else "diagnostic_only",
        "ineligibility_reasons": reasons,
        "artifacts": {
            "plan": artifact(plan_path),
            "combined_jsonl": artifact(records_path),
            "execution": execution,
            "state": state,
            "preflight": preflight,
        },
        "observed": {
            "records": len(records),
            "pair_count": len(pair_counts),
            "incomplete_pairs": incomplete_pairs,
            "invalid_pair_ids": invalid_pair_ids,
            "invalid_results": invalid_results,
            "outcomes": dict(sorted(outcomes.items())),
        },
        "claims": {
            "strength": "not_established" if not formal else "requires_finalized_formal_gate",
            "historical_result": "diagnostic_only" if not formal else "formal_candidate",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = audit(args.run_dir)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {document['classification']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
