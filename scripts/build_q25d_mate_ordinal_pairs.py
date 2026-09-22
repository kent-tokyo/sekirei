#!/usr/bin/env python3
"""Build fail-closed cp or mate-ordinal ranking pairs from Q25 label rows.

Mate values are deliberately never converted to centipawns.  A mate-only
candidate set can contribute only strict adjacent ordinal pairs, and only when
all available mate labels have one sign.  Mixed cp/mate and mixed-sign mate
sets stay in the audit as exclusions for a later protocol, rather than adding
an arbitrary numerical scale to the trainer input.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from prepare_q25_external_teacher_calibration import bind, sha256


NORMAL_SCORE_ABS_MAX_CP = 10_000


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def label_scores(row: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        move: label["score"]
        for move, label in row.get("labels", {}).items()
        if label.get("status") == "labeled" and isinstance(label.get("score"), dict)
    }


def mate_key(score: dict[str, Any]) -> tuple[int, int]:
    """Sort a positive mate sooner, and a negative mate later, ahead first."""
    value = score.get("value")
    require(isinstance(value, int) and value != 0, "mate value must be nonzero integer")
    if value > 0:
        return (0, value)
    return (1, value)


def terminal_rank_key(score: dict[str, Any]) -> tuple[int, int]:
    """Order decisive mates above cp scores and losing mates below them."""
    kind = score.get("kind")
    value = score.get("value")
    require(isinstance(value, int), "label score must be an integer")
    if kind == "mate":
        return mate_key(score)
    require(kind == "cp" and abs(value) <= NORMAL_SCORE_ABS_MAX_CP, "invalid cp label")
    return (1, -value)


def mixed_terminal_pairs(scores: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    """Build ordinal pairs without inventing a cp distance for a mate score."""
    ranked = sorted(scores.items(), key=lambda item: (terminal_rank_key(item[1]), item[0]))
    pairs = []
    for (high_move, high_score), (low_move, low_score) in zip(ranked, ranked[1:]):
        if high_score == low_score:
            continue
        if high_score.get("kind") == low_score.get("kind") == "cp":
            pairs.append(
                {
                    "higher_move_usi": high_move,
                    "lower_move_usi": low_move,
                    "label_kind": "centipawn",
                    "teacher_score_gap_cp": int(high_score["value"]) - int(low_score["value"]),
                }
            )
        else:
            pairs.append(
                {
                    "higher_move_usi": high_move,
                    "lower_move_usi": low_move,
                    "label_kind": "terminal_ordinal",
                    "teacher_order_margin": 1,
                }
            )
    return pairs


def adjacent_pairs(
    row: dict[str, Any], allow_mixed_terminal_ordinal: bool = False
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    scores = label_scores(row)
    kinds = {score.get("kind") for score in scores.values()}
    if len(scores) < 2:
        return [], {"excluded": True, "reason": "fewer_than_two_labeled_moves"}
    cp_scores = {
        move: score
        for move, score in scores.items()
        if score.get("kind") == "cp"
        and isinstance(score.get("value"), int)
        and abs(int(score["value"])) <= NORMAL_SCORE_ABS_MAX_CP
    }
    if allow_mixed_terminal_ordinal and kinds <= {"cp", "mate"} and {"cp", "mate"} <= kinds:
        terminal_scores = {
            move: score
            for move, score in scores.items()
            if score.get("kind") == "mate"
            or (
                score.get("kind") == "cp"
                and isinstance(score.get("value"), int)
                and abs(int(score["value"])) <= NORMAL_SCORE_ABS_MAX_CP
            )
        }
        terminal_kinds = {score.get("kind") for score in terminal_scores.values()}
        if terminal_kinds == {"cp", "mate"}:
            pairs = mixed_terminal_pairs(terminal_scores)
        elif terminal_kinds == {"mate"}:
            signs = {int(score["value"]) > 0 for score in terminal_scores.values()}
            ranked = sorted(terminal_scores.items(), key=lambda item: (mate_key(item[1]), item[0]))
            pairs = [
                {
                    "higher_move_usi": high_move,
                    "lower_move_usi": low_move,
                    "label_kind": "mate_ordinal",
                    "teacher_order_margin": 1,
                }
                for (high_move, high_score), (low_move, low_score) in zip(ranked, ranked[1:])
                if len(signs) == 1 and high_score["value"] != low_score["value"]
            ]
        else:
            pairs = []
        return pairs, {
            "excluded": not bool(pairs),
            "reason": None if pairs else "all_mixed_terminal_scores_tied",
            "mixed_terminal_ordinal": True,
            "omitted_out_of_range_cp_moves": sorted(set(scores) - set(terminal_scores)),
        }
    if kinds == {"cp"} or (kinds == {"cp", "mate"} and len(cp_scores) >= 2):
        ranked = sorted(cp_scores.items(), key=lambda item: (-int(item[1]["value"]), item[0]))
        pairs = [
            {
                "higher_move_usi": high_move,
                "lower_move_usi": low_move,
                "label_kind": "centipawn",
                "teacher_score_gap_cp": int(high_score["value"]) - int(low_score["value"]),
            }
            for (high_move, high_score), (low_move, low_score) in zip(ranked, ranked[1:])
            if int(high_score["value"]) > int(low_score["value"])
        ]
        return pairs, {
            "excluded": not bool(pairs),
            "reason": None if pairs else "fewer_than_two_ordinary_cp_scores_or_all_tied",
            "omitted_mate_moves": sorted(set(scores) - set(cp_scores)),
        }
    if kinds != {"mate"}:
        return [], {"excluded": True, "reason": "mixed_cp_and_mate_scores_without_two_cp_moves"}
    signs = {int(score["value"]) > 0 for score in scores.values()}
    if len(signs) != 1:
        return [], {"excluded": True, "reason": "mixed_mate_signs"}
    ranked = sorted(scores.items(), key=lambda item: (mate_key(item[1]), item[0]))
    pairs = [
        {
            "higher_move_usi": high_move,
            "lower_move_usi": low_move,
            "label_kind": "mate_ordinal",
            "teacher_order_margin": 1,
        }
        for (high_move, high_score), (low_move, low_score) in zip(ranked, ranked[1:])
        if high_score["value"] != low_score["value"]
    ]
    return pairs, {"excluded": not bool(pairs), "reason": None if pairs else "all_mate_scores_tied"}


def build_row(
    row: dict[str, Any], allow_mixed_terminal_ordinal: bool = False
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    pairs, audit = adjacent_pairs(row, allow_mixed_terminal_ordinal)
    common = {
        key: row[key]
        for key in ("id", "category", "initial_sfen", "history_before_usi", "sfen", "source")
        if key in row
    }
    return [{**common, **pair} for pair in pairs], {"parent_id": row.get("id"), **audit, "pairs": len(pairs)}


def frozen_pair_semantics(prereg: dict[str, Any]) -> dict[str, Any]:
    """Read optional pair semantics from the family bound before selection.

    The execution preregistration binds the frozen family by path and SHA. It
    intentionally does not duplicate arbitrary family fields, so a later
    pair-builder must verify that binding rather than accepting a CLI switch
    after labels are visible.
    """
    family_binding = prereg.get("family")
    if not isinstance(family_binding, dict):
        return {}
    path_value = family_binding.get("path")
    digest = family_binding.get("sha256")
    require(isinstance(path_value, str) and isinstance(digest, str), "invalid frozen family binding")
    family_path = Path(path_value)
    require(family_path.is_file() and sha256(family_path) == digest, "frozen family SHA mismatch")
    family = json.loads(family_path.read_text(encoding="utf-8"))
    require(
        family.get("schema") == "sekirei.q25a-external-label-family-preregistration.v1"
        and family.get("status") == "frozen_before_score_blind_parent_selection",
        "unexpected frozen family contract",
    )
    semantics = family.get("pair_semantics", {})
    require(isinstance(semantics, dict), "pair semantics must be an object")
    return semantics


def build(preregistration_path: Path, labels_path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    prereg = json.loads(preregistration_path.read_text(encoding="utf-8"))
    labels = json.loads(labels_path.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q25a-external-label-execution-preregistration.v1"
        and prereg.get("status") == "frozen_before_any_external_or_self_label",
        "unexpected Q25 execution preregistration",
    )
    require(
        labels.get("schema") == "sekirei.q25a-external-label-measurements.v2"
        and labels.get("status") == "complete"
        and labels.get("kind") in {"train", "holdout"}
        and labels.get("preregistration", {}).get("sha256") == sha256(preregistration_path),
        "labels are not a complete bound Q25 train measurement",
    )
    rows = labels.get("rows", [])
    pair_semantics = frozen_pair_semantics(prereg)
    allow_mixed_terminal_ordinal = bool(pair_semantics.get("allow_mixed_terminal_ordinal", False))
    kind = labels["kind"]
    expected = prereg["selection"][f"{kind}_parents"]
    require(len(rows) == expected, "label row count does not match the frozen train reserve")
    pairs: list[dict[str, Any]] = []
    audits: list[dict[str, Any]] = []
    for row in rows:
        row_pairs, audit = build_row(row, allow_mixed_terminal_ordinal)
        audits.append(audit)
        for pair in row_pairs:
            pairs.append(
                {
                    "parent_id": pair.pop("id"),
                    "category": pair.pop("category"),
                    "initial_sfen": pair.pop("initial_sfen"),
                    "history_before_usi": pair.pop("history_before_usi"),
                    "parent_sfen": pair.pop("sfen"),
                    **pair,
                }
            )
    complete = len(audits) == expected and all(not audit["excluded"] for audit in audits)
    external = prereg["external_teacher"]
    corpus = {
        "schema": "sekirei.root-rank-pairs.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "source_contract": {
            "depth": prereg["candidate_contract"]["external_depth"],
            "threads": external["threads"],
            "spec_top_n": 0,
            "root_candidate_mode": "preregistered_candidate_union",
            "root_candidate_limit": prereg["candidate_contract"]["maximum_moves_per_parent"],
            "complete_legal_root_set": False,
            "candidate_source_sha256": sha256(labels_path),
            "per_category_unique_positions": 1,
            "normal_score_abs_max_cp": prereg["candidate_contract"]["normal_score_abs_max_cp"],
        },
        "source_teacher": {
            "binary": prereg["inputs"]["external_engine"]["path"],
            "binary_sha256": prereg["inputs"]["external_engine"]["sha256"],
            "weights": prereg["inputs"]["external_weights"]["path"],
            "weights_sha256": prereg["inputs"]["external_weights"]["sha256"],
            "nnue_output": "residual-material",
        },
        "pair_selection": "adjacent",
        "pairs": pairs,
    }
    audit = {
        "schema": "sekirei.q25d-mate-ordinal-pair-coverage.v1",
        "status": (
            "candidate_training_authorized" if complete and kind == "train"
            else "holdout_screen_authorized" if complete
            else "fail_insufficient_strict_pair_coverage"
        ),
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_trained": False,
        "candidate_adopted": False,
        "q20_authorized": False,
        "holdout_inspected": False,
        "preregistration": bind(preregistration_path),
        "train_labels": bind(labels_path),
        "coverage": {
            "kind": kind,
            "processed_parents": len(audits),
            "parents_with_strict_pairs": sum(not row["excluded"] for row in audits),
            "strict_pairs": len(pairs),
        },
        "rows": audits,
    }
    return corpus, audit


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--labels", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--audit-output", type=Path, required=True)
    parser.add_argument("--cp-only-output", type=Path)
    args = parser.parse_args()
    try:
        corpus, audit = build(args.preregistration, args.labels)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(corpus, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.cp_only_output:
        cp_only = dict(corpus)
        cp_only["pairs"] = [
            pair for pair in corpus["pairs"] if pair.get("label_kind", "centipawn") == "centipawn"
        ]
        require(cp_only["pairs"], "no centipawn pairs are available for the static diagnostic")
        args.cp_only_output.write_text(
            json.dumps(cp_only, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    args.audit_output.write_text(json.dumps(audit, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": audit["status"], **audit["coverage"]}, sort_keys=True))
    return 0 if audit["status"] in {"candidate_training_authorized", "holdout_screen_authorized"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
