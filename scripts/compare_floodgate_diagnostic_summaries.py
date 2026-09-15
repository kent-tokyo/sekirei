#!/usr/bin/env python3
"""Compare two diagnostic summaries without turning them into a strength test."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from validate_floodgate_diagnostic_summary import validate


SCHEMA = "sekirei.floodgate-diagnostic-candidate-comparison.v1"
COMPARABLE_CLASSES = {"root_score_gap_observed", "tt_move_change_observed", "no_difference_observed"}


def key(row: dict) -> tuple[str, int]:
    game_id = row.get("game_id")
    ply = row.get("ply")
    if not isinstance(game_id, str) or not game_id or not isinstance(ply, int):
        raise ValueError("each row requires a non-empty game_id and integer ply")
    return game_id, ply


def index_rows(document: dict, label: str) -> dict[tuple[str, int], dict]:
    errors = validate(document)
    if errors:
        raise ValueError(f"{label} is invalid: {', '.join(errors)}")
    indexed = {}
    for row in document["rows"]:
        row_key = key(row)
        if row_key in indexed:
            raise ValueError(f"{label} has duplicate row key: {row_key[0]}:{row_key[1]}")
        indexed[row_key] = row
    return indexed


def execution_differences(baseline: dict, candidate: dict) -> list[str]:
    before = baseline.get("execution")
    after = candidate.get("execution")
    if not isinstance(before, dict) or not isinstance(after, dict):
        return ["execution_metadata_missing"]
    differences = []
    if before.get("source_revision") != after.get("source_revision"):
        differences.append("source_revision")
    for artifact in ("binary", "weights"):
        before_hash = before.get(artifact, {}).get("sha256") if isinstance(before.get(artifact), dict) else None
        after_hash = after.get(artifact, {}).get("sha256") if isinstance(after.get(artifact), dict) else None
        if before_hash != after_hash:
            differences.append(f"{artifact}.sha256")
    before_options = before.get("options", {})
    after_options = after.get("options", {})
    for option in sorted(set(before_options) | set(after_options)):
        if before_options.get(option) != after_options.get(option):
            differences.append(f"options.{option}")
    if before.get("corpus_sha256") != after.get("corpus_sha256"):
        differences.append("corpus_sha256")
    for field in ("nodes", "warmup_nodes"):
        if baseline.get(field) != candidate.get(field):
            differences.append(field)
    return differences


def corpus_hash(document: dict) -> str | None:
    execution = document.get("execution")
    if isinstance(execution, dict) and isinstance(execution.get("corpus_sha256"), str):
        return execution["corpus_sha256"]
    return document.get("source_corpus_sha256") if isinstance(document.get("source_corpus_sha256"), str) else None


def root_candidate_scores(row: dict) -> dict[str, int]:
    candidates = row.get("root_candidates")
    if not isinstance(candidates, list):
        return {}
    return {candidate["move"]: candidate["score_cp"] for candidate in candidates}


def compare(baseline: dict, candidate: dict, allowed_execution_differences: set[str] | None = None) -> dict:
    base = index_rows(baseline, "baseline")
    cand = index_rows(candidate, "candidate")
    base_keys = set(base)
    candidate_keys = set(cand)
    if base_keys != candidate_keys:
        missing = sorted(base_keys - candidate_keys)
        extra = sorted(candidate_keys - base_keys)
        raise ValueError(f"row key mismatch: missing={missing!r}, extra={extra!r}")
    metadata_differences = execution_differences(baseline, candidate)
    baseline_corpus = corpus_hash(baseline)
    candidate_corpus = corpus_hash(candidate)
    allowed = allowed_execution_differences or set()
    disallowed_metadata_differences = [difference for difference in metadata_differences if difference not in allowed]

    rows = []
    for row_key in sorted(base_keys):
        before = base[row_key]
        after = cand[row_key]
        complete = (
            not disallowed_metadata_differences
            and before["diagnostic_class"] in COMPARABLE_CLASSES
            and after["diagnostic_class"] in COMPARABLE_CLASSES
            and not before["unrestricted_aborted"]
            and not before["actual_root_aborted"]
            and not after["unrestricted_aborted"]
            and not after["actual_root_aborted"]
            and before["unrestricted_bound"] == "exact"
            and before["actual_root_bound"] == "exact"
            and after["unrestricted_bound"] == "exact"
            and after["actual_root_bound"] == "exact"
        )
        rows.append({
            "game_id": row_key[0],
            "ply": row_key[1],
            "status": "comparable" if complete else "not_comparable",
            "not_comparable_reasons": disallowed_metadata_differences,
            "baseline_class": before["diagnostic_class"],
            "candidate_class": after["diagnostic_class"],
            "baseline_unrestricted_bestmove": before.get("unrestricted_bestmove"),
            "candidate_unrestricted_bestmove": after.get("unrestricted_bestmove"),
            "bestmove_changed": before.get("unrestricted_bestmove") != after.get("unrestricted_bestmove"),
            "baseline_score_delta_cp": before.get("score_delta_actual_minus_unrestricted_cp"),
            "candidate_score_delta_cp": after.get("score_delta_actual_minus_unrestricted_cp"),
            "baseline_root_candidate_moves": sorted(root_candidate_scores(before)),
            "candidate_root_candidate_moves": sorted(root_candidate_scores(after)),
            "root_candidate_set_changed": set(root_candidate_scores(before)) != set(root_candidate_scores(after)),
            "root_candidate_scores_changed": root_candidate_scores(before) != root_candidate_scores(after),
            "root_candidate_score_deltas_cp": {
                move: root_candidate_scores(after)[move] - root_candidate_scores(before)[move]
                for move in sorted(set(root_candidate_scores(before)) & set(root_candidate_scores(after)))
            },
        })
    comparable = sum(row["status"] == "comparable" for row in rows)
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "baseline_schema": baseline["schema"],
        "candidate_schema": candidate["schema"],
        "row_key": "game_id+ply",
        "execution_differences": metadata_differences,
        "baseline_corpus_sha256": baseline_corpus,
        "candidate_corpus_sha256": candidate_corpus,
        "source_corpus_sha256": baseline_corpus if baseline_corpus == candidate_corpus else None,
        "allowed_execution_differences": sorted(allowed),
        "rows": rows,
        "counts": {"total": len(rows), "comparable": comparable, "not_comparable": len(rows) - comparable},
        "decision": "not_evaluable",
        "claims": {
            "strength": "not_permitted",
            "played_move_is_label": False,
            "interpretation": "candidate differences are diagnostic only; independent strength evidence is required",
        },
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--allow-execution-difference", action="append", default=[],
                        help="one declared ablation metadata path; may be repeated")
    args = parser.parse_args(argv)
    try:
        result = compare(json.loads(args.baseline.read_text(encoding="utf-8")),
                         json.loads(args.candidate.read_text(encoding="utf-8")),
                         set(args.allow_execution_difference))
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        parser.error(str(exc))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['counts']['comparable']}/{result['counts']['total']} comparable")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
