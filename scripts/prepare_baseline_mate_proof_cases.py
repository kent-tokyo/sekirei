#!/usr/bin/env python3
"""Turn complete, legal baseline searches into bounded opponent-mate probes.

The output is diagnostic input, never a best-move label. It binds every move
to its original case position and rejects partial or non-exact searches.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from run_tactical_line_diagnostic import REPORT_SCHEMA, SCHEMA, load_cases, sha256


def make_cases(cases_path: Path, report_path: Path, nodes: int) -> dict:
    cases = load_cases(cases_path)
    report = json.loads(report_path.read_text(encoding="utf-8"))
    if report.get("schema") != REPORT_SCHEMA or report.get("status") != "complete":
        raise ValueError("baseline report must be complete")
    if report.get("inputs", {}).get("cases_sha256") != sha256(cases_path):
        raise ValueError("baseline report does not bind the supplied cases")
    if report.get("inputs", {}).get("evaluator") != "material":
        raise ValueError("mate screen requires the material baseline")
    contract = report.get("contract", {})
    if contract.get("threads") != 1 or contract.get("spec_top_n") != 0:
        raise ValueError("baseline must be one thread without speculation")
    expected = {}
    for case in cases["cases"]:
        for stage in case["stages"]:
            if stage.get("moves") or stage.get("expected_sfen", case["sfen"]) != case["sfen"]:
                raise ValueError("only direct, unreplayed positions are supported")
            expected[(case["id"], stage["id"])] = case
    selected = {}
    recovered_completed = []
    excluded = []
    seen = set()
    for row in report.get("rows", []):
        key = (row.get("case_id"), row.get("stage_id"))
        if row.get("nodes_per_invocation") != nodes or key not in expected:
            continue
        if key in seen:
            raise ValueError(f"duplicate baseline row for {key}")
        seen.add(key)
        if row.get("position_sfen") != expected[key]["sfen"] or row.get("moves"):
            raise ValueError(f"position mismatch for {key}")
        result = row.get("unrestricted", {})
        returned_move = result.get("bestmove")
        pv = result.get("pv_usi")
        pv_root = pv[0] if isinstance(pv, list) and pv else None
        returned_matches_pv = returned_move == pv_root
        completed_move = result.get("completed_bestmove")
        recover_completed = (not returned_matches_pv and result.get("aborted") is True
                             and result.get("bestmove_matches_pv_root") is False
                             and completed_move == pv_root)
        move = completed_move if recover_completed else returned_move
        if (not isinstance(move, str) or not move or move in ("resign", "win", "none")
                or result.get("completed_iteration_valid") is not True
                or result.get("completed_bound") != "exact"
                or result.get("pv_legal") is not True
                or not isinstance(pv, list) or not pv
                or pv_root != move
                or result.get("pv_root", pv_root) != pv_root
                or (not returned_matches_pv and not recover_completed)
                or (returned_matches_pv and result.get("bestmove_matches_pv_root", True) is not True)
                or (completed_move is not None and completed_move != pv_root)):
            excluded.append({"case_id": key[0], "stage_id": key[1],
                             "reason": "unqualified_or_inconsistent_baseline_move"})
            continue
        selected[key] = move
        if recover_completed:
            recovered_completed.append({"case_id": key[0], "stage_id": key[1],
                                        "returned_bestmove": returned_move,
                                        "completed_bestmove": move})
    if seen != set(expected):
        raise ValueError("baseline report lacks one exact row per case and stage")
    if not selected:
        raise ValueError("no qualified baseline moves")
    output = []
    for key, case in expected.items():
        if key not in selected:
            continue
        output.append({
            "id": f"{key[0]}-{key[1]}-baseline-proof",
            "sfen": case["sfen"],
            "stages": [{"id": "baseline", "kind": "diagnostic_baseline_move",
                        "moves": [], "expected_sfen": case["sfen"],
                        "candidates": [selected[key]], "reference_moves": []}],
        })
    return {"schema": SCHEMA, "diagnostic_only": True, "strength_claim": False,
            "source_cases_sha256": sha256(cases_path),
            "source_report_sha256": sha256(report_path),
            "nodes_per_baseline_search": nodes, "excluded": excluded,
            "recovered_completed_moves": recovered_completed,
            "cases": output}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--nodes", required=True, type=int)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if not 1 <= args.nodes <= 100_000:
        parser.error("--nodes must be 1..=100000")
    document = make_cases(args.cases, args.report, args.nodes)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(document, output, ensure_ascii=False, indent=2, sort_keys=True)
        output.write("\n")
    print(json.dumps({"cases": len(document["cases"]),
                      "excluded": len(document["excluded"]), "output": str(args.output)}))


if __name__ == "__main__":
    main()
