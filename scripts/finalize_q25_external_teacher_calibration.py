#!/usr/bin/env python3
"""Validate Q25 A/A evidence and decide whether external labels merit a new family."""

from __future__ import annotations

import argparse
import json
import statistics
from pathlib import Path
from typing import Any

from prepare_q25_external_teacher_calibration import SCHEMA, bind, sha256


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def self_score(result: dict[str, Any]) -> dict[str, Any]:
    value = int(result["score_cp"])
    return {"kind": "mate" if abs(value) > 10_000 else "cp", "value": value}


def external_score(result: dict[str, Any]) -> dict[str, Any]:
    return result["lines"][0]["score"]


def external_score_for_move(result: dict[str, Any], move: str) -> dict[str, Any] | None:
    return next(
        (line["score"] for line in result.get("lines", []) if line.get("move") == move),
        None,
    )


def score_key(score: dict[str, Any]) -> tuple[int, int]:
    kind, value = score["kind"], int(score["value"])
    if kind == "cp":
        return (1, value)
    if value > 0:
        return (2, -abs(value))
    return (0, abs(value))


def stable(results: list[dict[str, Any]], score_reader: Any) -> bool:
    if len(results) < 2 or any(result.get("completion") != "search_completed" for result in results):
        return False
    first = (results[0].get("bestmove"), score_reader(results[0]))
    return all((result.get("bestmove"), score_reader(result)) == first for result in results[1:])


def direct_regret(top: dict[str, Any], alternative: dict[str, Any]) -> tuple[int | None, bool]:
    if top["kind"] == alternative["kind"] == "cp":
        return max(0, int(top["value"]) - int(alternative["value"])), False
    if top["kind"] == alternative["kind"] == "mate":
        return None, (int(top["value"]) > 0) != (int(alternative["value"]) > 0)
    return None, True


def top_moves_self(result: dict[str, Any]) -> list[str]:
    return [row["move"] for row in result.get("root_candidates", [])]


def top_moves_external(result: dict[str, Any]) -> list[str]:
    return [row["move"] for row in result.get("lines", [])]


def finalize(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    measurements = json.loads(args.measurements.read_text(encoding="utf-8"))
    require(prereg.get("schema") == SCHEMA, "unexpected preregistration")
    require(prereg.get("status") == "frozen_before_any_q25_label", "Q25 contract was not frozen")
    preregistered_finalizer_sha = prereg["tools"]["finalizer"]["sha256"]
    executed_finalizer_sha = sha256(Path(__file__).resolve())
    require(
        measurements.get("schema") == "sekirei.q25-external-teacher-measurements.v1"
        and measurements.get("preregistration", {}).get("sha256") == sha256(args.preregistration),
        "measurement contract mismatch",
    )
    summaries = []
    for row in measurements["rows"]:
        self_free_ok = stable(row["self_free"], self_score)
        external_free_ok = stable(row["external_free"], external_score)
        self_top = row["self_free"][0].get("bestmove")
        external_top = row["external_free"][0].get("bestmove")
        agreement = self_top == external_top
        self_cross_ok = agreement or stable(row["self_on_external_top"], self_score)
        external_free_alternatives = [
            external_score_for_move(result, self_top) for result in row["external_free"]
        ]
        external_free_alternative_ok = (
            not agreement
            and all(score is not None for score in external_free_alternatives)
            and len({json.dumps(score, sort_keys=True) for score in external_free_alternatives}) == 1
        )
        external_cross_ok = (
            agreement
            or external_free_alternative_ok
            or stable(row["external_on_self_top"], external_score)
        )
        valid = self_free_ok and external_free_ok and self_cross_ok and external_cross_ok
        external_regret = 0
        self_regret = 0
        mate_disagreement = False
        if valid and not agreement:
            external_alternative = (
                external_free_alternatives[0]
                if external_free_alternative_ok
                else external_score(row["external_on_self_top"][0])
            )
            external_regret, external_mate = direct_regret(
                external_score(row["external_free"][0]),
                external_alternative,
            )
            self_regret, self_mate = direct_regret(
                self_score(row["self_free"][0]),
                self_score(row["self_on_external_top"][0]),
            )
            mate_disagreement = external_mate or self_mate
        self_moves = set(top_moves_self(row["self_free"][0]))
        external_moves = set(top_moves_external(row["external_free"][0]))
        overlap = len(self_moves & external_moves) / len(self_moves | external_moves) if self_moves | external_moves else 0.0
        summaries.append(
            {
                "id": row["id"],
                "category": row["category"],
                "valid_aa": valid,
                "self_top": self_top,
                "external_top": external_top,
                "top1_agreement": agreement,
                "top_set_jaccard": overlap,
                "external_regret_of_self_top_cp": external_regret,
                "self_regret_of_external_top_cp": self_regret,
                "mate_class_disagreement": mate_disagreement,
                "external_self_move_score_source": (
                    "free_multipv" if external_free_alternative_ok else "forced_search"
                ) if not agreement else "top1_agreement",
            }
        )
    valid = [row for row in summaries if row["valid_aa"]]
    contract = prereg["measurement_contract"]
    require(len(valid) >= contract["minimum_valid_parents"], "fewer than preregistered valid A/A parents")
    cp_regrets = [
        row["external_regret_of_self_top_cp"]
        for row in valid
        if row["external_regret_of_self_top_cp"] is not None
    ]
    major = sum(
        regret >= contract["major_regret_cp"] for regret in cp_regrets
    )
    mate_disagreements = sum(row["mate_class_disagreement"] for row in valid)
    threshold = contract["external_family_warranted_if"]
    warranted = (
        major >= threshold["major_external_regret_count_at_least"]
        or mate_disagreements >= threshold["or_mate_class_disagreement_count_at_least"]
    )
    result = {
        "schema": "sekirei.q25-external-teacher-calibration-decision.v1",
        "status": "external_teacher_candidate_family_warranted" if warranted else "self_teacher_not_disqualified",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_trained": False,
        "candidate_adopted": False,
        "q20_authorized": False,
        "competitor_match_authorized": False,
        "preregistration": bind(args.preregistration),
        "measurements": bind(args.measurements),
        "parents_total": len(summaries),
        "parents_valid_aa": len(valid),
        "top1_agreements": sum(row["top1_agreement"] for row in valid),
        "top1_agreement_rate": sum(row["top1_agreement"] for row in valid) / len(valid),
        "mean_top_set_jaccard": statistics.fmean(row["top_set_jaccard"] for row in valid),
        "ordinary_cp_external_regret_count": len(cp_regrets),
        "mean_external_regret_of_self_top_cp": statistics.fmean(cp_regrets) if cp_regrets else None,
        "major_external_regret_ge_300_count": major,
        "mate_class_disagreement_count": mate_disagreements,
        "external_teacher_family_warranted": warranted,
        "interpretation": (
            "The external teacher is a calibration reference, not an oracle or a strength result."
        ),
        "tool_provenance": {
            "preregistered_sha256": preregistered_finalizer_sha,
            "executed_sha256": executed_finalizer_sha,
            "contract_preserving_fix_after_measurement": (
                preregistered_finalizer_sha != executed_finalizer_sha
            ),
            "fix_scope": (
                "use the preregistered A/A-stable free MultiPV score when it already contains "
                "the self-teacher move; only fall back to forced search outside that set"
            ),
        },
        "next_action": (
            "preregister a new score-blind train/holdout split for one external-label candidate family"
            if warranted
            else "retain the self depth-7 teacher and continue to Q26 search-cost profiling"
        ),
        "rows": summaries,
        "finalizer": bind(Path(__file__).resolve()),
    }
    args.output.write_text(
        json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--measurements", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = finalize(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({key: result[key] for key in ("status", "parents_valid_aa", "top1_agreement_rate", "major_external_regret_ge_300_count", "mate_class_disagreement_count", "next_action")}, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
