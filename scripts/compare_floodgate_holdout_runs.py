#!/usr/bin/env python3
"""Compare two core diagnostic runs on unlabeled hold-out positions."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from compare_floodgate_diagnostic_summaries import execution_differences

SCHEMA = "sekirei.floodgate-holdout-comparison.v1"


def rows(document: dict, label: str) -> dict[str, dict]:
    if document.get("diagnostic_only") is not True:
        raise ValueError(f"{label} is not diagnostic-only")
    indexed = {}
    for index, item in enumerate(document.get("results", [])):
        game_id = item.get("source", {}).get("game_id")
        key = f"{game_id}:{index}"
        if not isinstance(game_id, str) or not game_id:
            raise ValueError(f"{label} result {index} lacks source.game_id")
        if key in indexed:
            raise ValueError(f"{label} has duplicate result key {key}")
        indexed[key] = item
    return indexed


def corpus_hash(document: dict) -> str | None:
    value = document.get("source_corpus_sha256")
    if isinstance(value, str):
        return value
    execution = document.get("execution", {})
    value = execution.get("corpus_sha256") if isinstance(execution, dict) else None
    return value if isinstance(value, str) else None


def search_integrity(result: dict) -> dict:
    """Classify explicit PV/board-preservation evidence without inferring it."""
    fields = ("pv_legal", "pv_replay_preserves_input")
    missing = [field for field in fields if field not in result]
    if missing:
        return {"status": "unknown", "missing": missing}
    values = {field: result[field] for field in fields}
    if any(not isinstance(value, bool) for value in values.values()):
        return {"status": "unknown", "missing": [], "invalid": list(values)}
    status = "verified" if all(values.values()) else "failed"
    return {"status": status, **values}


def compare(baseline: dict, candidate: dict, allowed_execution_differences: set[str] | None = None) -> dict:
    before = rows(baseline, "baseline")
    after = rows(candidate, "candidate")
    if set(before) != set(after):
        raise ValueError("hold-out result keys do not match")
    differences = execution_differences(baseline, candidate)
    allowed = allowed_execution_differences or set()
    disallowed = [difference for difference in differences if difference not in allowed]
    output = []
    for key in sorted(before):
        left = before[key].get("unrestricted", {})
        right = after[key].get("unrestricted", {})
        complete = all(
            result.get("completion") == "search_completed"
            and result.get("aborted") is False
            and result.get("bound") == "exact"
            for result in (left, right)
        ) and not disallowed
        output.append({
            "key": key,
            "status": "comparable" if complete else "not_comparable",
            "not_comparable_reasons": disallowed,
            "bestmove_changed": left.get("bestmove") != right.get("bestmove"),
            "pv_changed": left.get("pv_usi", []) != right.get("pv_usi", []),
            "score_delta_candidate_minus_baseline_cp": right.get("score_cp", 0) - left.get("score_cp", 0),
            "depth_delta_candidate_minus_baseline": right.get("depth", 0) - left.get("depth", 0),
            "baseline_search_integrity": search_integrity(left),
            "candidate_search_integrity": search_integrity(right),
        })
    comparable = sum(item["status"] == "comparable" for item in output)
    result = {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "rows": output,
        "counts": {"total": len(output), "comparable": comparable, "not_comparable": len(output) - comparable},
        "execution_differences": differences,
        "allowed_execution_differences": sorted(allowed),
        "decision": "not_evaluable",
        "claims": {"strength": "not_permitted", "played_move_is_label": False},
    }
    baseline_hash = corpus_hash(baseline)
    candidate_hash = corpus_hash(candidate)
    result["baseline_corpus_sha256"] = baseline_hash
    result["candidate_corpus_sha256"] = candidate_hash
    result["source_corpus_sha256"] = baseline_hash if baseline_hash == candidate_hash else None
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--allow-execution-difference", action="append", default=[])
    args = parser.parse_args(argv)
    result = compare(json.loads(args.baseline.read_text(encoding="utf-8")),
                     json.loads(args.candidate.read_text(encoding="utf-8")),
                     set(args.allow_execution_difference))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['counts']['comparable']}/{result['counts']['total']} comparable")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
