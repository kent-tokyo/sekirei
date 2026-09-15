#!/usr/bin/env python3
"""Classify fixed-depth root-ranking errors without selecting a candidate."""
from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path


SCHEMA = "sekirei.root-rank-pair-audit.v1"
GAP_BANDS = (("small", 0, 31), ("medium", 32, 127), ("large", 128, None))
SUPPORT_RULE = {
    "minimum_pairs_in_single_bucket": 12,
    "minimum_net_recoveries_in_single_bucket": 4,
    "maximum_candidate_regressions_in_single_bucket": 0,
    "global_candidate_decision": "must be eligible_for_next_diagnostic_only",
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def gap_band(gap: int) -> str:
    for name, lower, upper in GAP_BANDS:
        if gap >= lower and (upper is None or gap <= upper):
            return name
    raise ValueError(f"invalid teacher score gap {gap}")


def load_pairs(path: Path) -> dict[tuple[str, str, str], dict]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA or document.get("diagnostic_only") is not True:
        raise ValueError(f"{path}: unsupported ranking audit")
    diagnostic = document.get("model_diagnostic")
    if not isinstance(diagnostic, dict) or not isinstance(diagnostic.get("pair_diagnostics"), list):
        raise ValueError(f"{path}: pair diagnostics are required")
    pairs = {}
    for pair in diagnostic["pair_diagnostics"]:
        required = ("parent_id", "category", "teacher_score_gap_cp", "higher_move_usi", "lower_move_usi",
                    "parent_oriented_margin_cp", "teacher_order_preserved")
        if not isinstance(pair, dict) or any(key not in pair for key in required):
            raise ValueError(f"{path}: malformed pair diagnostic")
        if not isinstance(pair["teacher_score_gap_cp"], int) or pair["teacher_score_gap_cp"] <= 0:
            raise ValueError(f"{path}: invalid teacher score gap")
        if not isinstance(pair["parent_oriented_margin_cp"], int) or not isinstance(pair["teacher_order_preserved"], bool):
            raise ValueError(f"{path}: invalid pair score")
        key = (pair["parent_id"], pair["higher_move_usi"], pair["lower_move_usi"])
        if key in pairs:
            raise ValueError(f"{path}: duplicate pair diagnostic {key}")
        pairs[key] = pair
    if not pairs:
        raise ValueError(f"{path}: empty pair diagnostics")
    return pairs


def transition(baseline: bool, candidate: bool) -> str:
    if baseline and candidate:
        return "both_correct"
    if baseline and not candidate:
        return "candidate_regressed"
    if not baseline and candidate:
        return "candidate_recovered"
    return "both_incorrect"


def bucket_summary(rows: list[dict]) -> dict:
    states = Counter(row["transition"] for row in rows)
    return {
        "pairs": len(rows),
        "both_correct": states["both_correct"],
        "candidate_recovered": states["candidate_recovered"],
        "candidate_regressed": states["candidate_regressed"],
        "both_incorrect": states["both_incorrect"],
        "net_recoveries": states["candidate_recovered"] - states["candidate_regressed"],
        "mean_margin_delta_cp": sum(row["margin_delta_cp"] for row in rows) / len(rows),
    }


def analyze(baseline: dict, candidate: dict, expected_pairs: int, candidate_eligible: bool) -> dict:
    if len(baseline) != expected_pairs or len(candidate) != expected_pairs:
        raise ValueError("audit pair counts do not match frozen stable pair set")
    if baseline.keys() != candidate.keys():
        raise ValueError("baseline and candidate pair identities differ")
    rows = []
    for key in sorted(baseline):
        before, after = baseline[key], candidate[key]
        if before["category"] != after["category"] or before["teacher_score_gap_cp"] != after["teacher_score_gap_cp"]:
            raise ValueError(f"pair metadata differs for {key}")
        rows.append({
            "parent_id": before["parent_id"],
            "category": before["category"],
            "teacher_gap_band": gap_band(before["teacher_score_gap_cp"]),
            "teacher_score_gap_cp": before["teacher_score_gap_cp"],
            "transition": transition(before["teacher_order_preserved"], after["teacher_order_preserved"]),
            "margin_delta_cp": after["parent_oriented_margin_cp"] - before["parent_oriented_margin_cp"],
        })
    by_category = defaultdict(list)
    by_gap_band = defaultdict(list)
    for row in rows:
        by_category[row["category"]].append(row)
        by_gap_band[row["teacher_gap_band"]].append(row)
    summaries = {
        "category": {key: bucket_summary(value) for key, value in sorted(by_category.items())},
        "teacher_gap_band": {key: bucket_summary(value) for key, value in sorted(by_gap_band.items())},
    }
    supported = []
    for dimension, values in summaries.items():
        for bucket, summary in values.items():
            if (summary["pairs"] >= SUPPORT_RULE["minimum_pairs_in_single_bucket"]
                    and summary["net_recoveries"] >= SUPPORT_RULE["minimum_net_recoveries_in_single_bucket"]
                    and summary["candidate_regressed"] <= SUPPORT_RULE["maximum_candidate_regressions_in_single_bucket"]):
                supported.append(f"{dimension}:{bucket}")
    status = "one_factor_hypothesis_supported" if candidate_eligible and supported else "inconclusive_no_one_factor_hypothesis"
    return {
        "schema": "sekirei.root-ranking-error-analysis.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "gap_bands_cp": [{"name": name, "min": lower, "max": upper} for name, lower, upper in GAP_BANDS],
        "support_rule": SUPPORT_RULE,
        "candidate_eligible": candidate_eligible,
        "overall": bucket_summary(rows),
        "by": summaries,
        "supported_buckets": supported,
        "status": status,
        "next_action": ("select_one_factor_diagnostic" if status == "one_factor_hypothesis_supported"
                        else "do_not_change_model_or_search_from_this_comparison"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-audit", type=Path, required=True)
    parser.add_argument("--candidate-audit", type=Path, required=True)
    parser.add_argument("--stable-manifest", type=Path, required=True)
    parser.add_argument("--candidate-decision", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        stable = json.loads(args.stable_manifest.read_text(encoding="utf-8"))
        expected_pairs = stable.get("retained_pairs")
        if not isinstance(expected_pairs, int) or expected_pairs <= 0:
            raise ValueError("stable manifest lacks retained_pairs")
        decision = json.loads(args.candidate_decision.read_text(encoding="utf-8"))
        candidate_eligible = decision.get("status") == "eligible_for_next_diagnostic_only"
        result = analyze(load_pairs(args.baseline_audit), load_pairs(args.candidate_audit), expected_pairs, candidate_eligible)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    result["inputs"] = {name: {"path": str(path), "sha256": sha256(path)} for name, path in {
        "baseline_audit": args.baseline_audit, "candidate_audit": args.candidate_audit,
        "stable_manifest": args.stable_manifest, "candidate_decision": args.candidate_decision,
    }.items()}
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
