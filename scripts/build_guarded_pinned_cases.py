#!/usr/bin/env python3
"""Build a distinct, guarded unpin comparison without overwriting older cases."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

from build_handcrafted_strategy_cases import position
from run_strategy_piece_mobility import SCHEMA as MOBILITY_SCHEMA
from run_tactical_line_diagnostic import sha256


PIECES = {
    "focus": {"5h": "K", "1a": "k", "5g": "G", "5c": "r"},
    "contrast": {"5h": "K", "1a": "k", "5g": "G", "3c": "r"},
}
EXPECTED_BEFORE = {"focus": 1, "contrast": 5}
CANDIDATE = "5h4h"
ORIGIN = "5g"


def build(legal_bin: Path) -> tuple[dict, dict]:
    cases = []
    selections = []
    for label, pieces in PIECES.items():
        sfen = position(pieces, "-")
        result = subprocess.run([str(legal_bin.resolve())], input=sfen + "\n",
                                text=True, capture_output=True, check=True, timeout=5)
        lines = result.stdout.splitlines()
        if len(lines) != 1:
            raise ValueError("legal helper must return exactly one position line")
        legal = lines[0].split()
        before = sum(move.startswith(ORIGIN) for move in legal)
        if CANDIDATE not in legal or before != EXPECTED_BEFORE[label]:
            raise ValueError(f"guarded {label}: candidate or initial gold mobility changed")
        case_id = f"pinned-guarded-{label}"
        cases.append({"id": case_id, "sfen": sfen,
                      "source": {"kind": "original_composed_reduced_material",
                                 "variant_of": f"pinned-exchange-{label}",
                                 "theme": "guarded-unpin",
                                 "theme_is_hypothesis": True,
                                 "reference_is_oracle": False},
                      "stages": [{"id": "first-move", "kind": "candidate_comparison",
                                  "moves": [], "expected_sfen": sfen,
                                  "candidates": [CANDIDATE], "reference_moves": []}]})
        selections.append({"case_id": case_id, "candidate": CANDIDATE, "origin": ORIGIN})
    return ({"schema": "sekirei.tactical-line-cases.v1", "diagnostic_only": True,
             "legal_moves_sha256": sha256(legal_bin),
             "note": "Legal guarded-unpin candidates; value is unproven.", "cases": cases},
            {"schema": MOBILITY_SCHEMA, "selections": selections})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--legal-moves", required=True, type=Path)
    parser.add_argument("--cases-output", required=True, type=Path)
    parser.add_argument("--spec-output", required=True, type=Path)
    args = parser.parse_args()
    if not args.legal_moves.is_file():
        parser.error("legal-move helper does not exist")
    if (args.cases_output == args.spec_output or args.cases_output.exists()
            or args.spec_output.exists()):
        parser.error("output paths must be distinct and new")
    cases, spec = build(args.legal_moves)
    for path, document in ((args.cases_output, cases), (args.spec_output, spec)):
        with path.open("x", encoding="utf-8") as output:
            json.dump(document, output, ensure_ascii=False, indent=2)
            output.write("\n")
    print(json.dumps({"cases": len(cases["cases"]), "spec": len(spec["selections"])}))


if __name__ == "__main__":
    main()
