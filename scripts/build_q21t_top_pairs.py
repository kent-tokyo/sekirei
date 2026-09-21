#!/usr/bin/env python3
"""Build direct teacher-top-versus-rest pairs from audited depth-7 labels."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import build_q21p_depth7_pairs as base


def build(
    preregistration_path: Path, measurements_path: Path
) -> tuple[dict[str, Any], dict[str, Any]]:
    prereg = json.loads(preregistration_path.read_text(encoding="utf-8"))
    measurements = json.loads(measurements_path.read_text(encoding="utf-8"))
    base.require(
        measurements.get("schema") == "sekirei.q21p-depth7-label-measurements.v1",
        "unexpected depth-7 measurement schema",
    )
    base.require(
        measurements.get("preregistration", {}).get("sha256")
        == base.sha256(preregistration_path),
        "measurements are not bound to the preregistration",
    )
    contract = prereg["candidate_contract"]
    teacher = prereg["teacher_contract"]
    rows = measurements.get("rows", [])
    base.require(len(rows) == prereg["parents"], "parent count mismatch")
    pairs: list[dict[str, Any]] = []
    audit_rows = []
    for row in rows:
        identifier = row["id"]
        free = row.get("free", [])
        fixed = row.get("fixed", [])
        base.require(
            len(free) == contract["free_repeats"]
            and len(fixed) == contract["fixed_root_repeats"],
            f"{identifier}: repeat count mismatch",
        )
        base.require(all(base.exact(item) for item in free), f"{identifier}: incomplete free search")
        base.require(
            all(base.signature(item) == base.signature(free[0]) for item in free[1:]),
            f"{identifier}: free-search A/A mismatch",
        )
        candidates = row.get("candidate_moves", [])
        base.require(
            all(set(repeat) == set(candidates) for repeat in fixed),
            f"{identifier}: fixed-root candidate set mismatch",
        )
        scores: dict[str, int] = {}
        for move in candidates:
            results = [repeat[move] for repeat in fixed]
            base.require(all(base.exact(item) for item in results), f"{identifier}/{move}: incomplete")
            base.require(
                all(base.signature(item) == base.signature(results[0]) for item in results[1:]),
                f"{identifier}/{move}: fixed-search A/A mismatch",
            )
            if base.ordinary(results[0], contract["normal_score_abs_max_cp"]):
                scores[move] = results[0]["score_cp"]
        if len(scores) < 2:
            audit_rows.append(
                {
                    "parent_id": identifier,
                    "category": row["category"],
                    "ordinary_moves": len(scores),
                    "excluded": True,
                    "reason": "fewer_than_two_ordinary_depth7_scores",
                }
            )
            continue
        ranked = sorted(scores.items(), key=lambda item: (-item[1], item[0]))[
            : contract["top_k_after_depth7_reranking"]
        ]
        best_score = ranked[0][1]
        top_moves = [move for move, score in ranked if score == best_score]
        lower_moves = [(move, score) for move, score in ranked if score < best_score]
        base.require(lower_moves, f"{identifier}: no strict top-versus-rest pair")
        parent_pairs = 0
        for top_move in top_moves:
            for lower_move, lower_score in lower_moves:
                pairs.append(
                    {
                        "parent_id": identifier,
                        "category": row["category"],
                        "initial_sfen": row["initial_sfen"],
                        "history_before_usi": row["history_before_usi"],
                        "parent_sfen": row["sfen"],
                        "source": row.get("source"),
                        "higher_move_usi": top_move,
                        "lower_move_usi": lower_move,
                        "teacher_score_gap_cp": best_score - lower_score,
                    }
                )
                parent_pairs += 1
        audit_rows.append(
            {
                "parent_id": identifier,
                "category": row["category"],
                "ordinary_moves": len(scores),
                "ranked_moves": len(ranked),
                "teacher_top_moves": top_moves,
                "top_score_cp": best_score,
                "direct_pairs": parent_pairs,
                "excluded": False,
            }
        )
    base.require(pairs, "no top-versus-rest pairs were generated")
    measurement_sha = base.sha256(measurements_path)
    output = {
        "schema": "sekirei.root-rank-pairs.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "source_contract": {
            "depth": teacher["max_depth"],
            "threads": teacher["threads"],
            "spec_top_n": teacher["spec_top_n"],
            "root_candidate_mode": "preregistered_candidate_union",
            "root_candidate_limit": contract["maximum_moves_per_parent"],
            "complete_legal_root_set": False,
            "candidate_source_sha256": measurement_sha,
            "per_category_unique_positions": 1,
            "normal_score_abs_max_cp": contract["normal_score_abs_max_cp"],
        },
        "source_teacher": {
            "binary": prereg["inputs"]["engine"]["path"],
            "binary_sha256": teacher["binary_sha256"],
            "weights": prereg["inputs"]["weights"]["path"],
            "weights_sha256": teacher["weights_sha256"],
            "nnue_output": teacher["nnue_output"],
        },
        "pair_selection": "top-vs-rest",
        "pairs": pairs,
    }
    audit = {
        "schema": "sekirei.q21t-top-vs-rest-pair-audit.v1",
        "status": "pass",
        "diagnostic_only": True,
        "strength_claim": False,
        "parents": len(rows),
        "parents_with_pairs": sum(not row["excluded"] for row in audit_rows),
        "parents_excluded": sum(row["excluded"] for row in audit_rows),
        "pairs": len(pairs),
        "regret_semantics": "teacher_score_gap_cp is the direct teacher-top minus lower-move score",
        "inputs": {
            "preregistration": {
                "path": str(preregistration_path),
                "sha256": base.sha256(preregistration_path),
            },
            "measurements": {"path": str(measurements_path), "sha256": measurement_sha},
            "builder": {
                "path": str(Path(__file__).resolve()),
                "sha256": base.sha256(Path(__file__).resolve()),
            },
        },
        "rows": audit_rows,
    }
    return output, audit


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--measurements", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--audit-output", type=Path, required=True)
    args = parser.parse_args()
    try:
        pairs, audit = build(args.preregistration, args.measurements)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(pairs, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    args.audit_output.write_text(
        json.dumps(audit, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "status": audit["status"],
                "parents": audit["parents_with_pairs"],
                "pairs": audit["pairs"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
