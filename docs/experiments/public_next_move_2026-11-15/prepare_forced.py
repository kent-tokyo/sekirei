#!/usr/bin/env python3
"""Select up to four preregistered development mismatches for fixed-root diagnosis."""

import argparse
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    cases = {case["id"]: case for case in json.loads(args.cases.read_text())["cases"]}
    baseline = json.loads(args.baseline.read_text())
    selected = {}
    for row in baseline["rows"]:
        case = cases[row["case_id"]]
        if case["split"] != "development":
            raise ValueError("confirmation case appeared in baseline")
        if row["bestmove"] != case["reference_first_move"]:
            selected.setdefault(case["id"], set()).update(
                (row["bestmove"], case["reference_first_move"])
            )
    result = {"schema": "sekirei.tactical-line-cases.v1", "cases": []}
    for case_id, candidates in list(selected.items())[:4]:
        case = cases[case_id]
        result["cases"].append({
            "id": case_id,
            "sfen": case["initial_sfen"],
            "stages": [{"id": "puzzle", "kind": "observation",
                        "moves": [case["initial_usi"]],
                        "expected_sfen": case["sfen"],
                        "candidates": sorted(candidates),
                        "reference_moves": [case["reference_first_move"]]}],
        })
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"case_ids": list(selected)[:4], "output": str(args.output)}))


if __name__ == "__main__":
    main()
