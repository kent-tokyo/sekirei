#!/usr/bin/env python3
"""Replay every recorded PV through a frozen Sekirei legal-move diagnostic.

This checks the engine's public PV contract against Sekirei's rule generator;
it is not an independent rule oracle or a mate-score proof.
"""
import argparse
import json
from pathlib import Path

from generate_search_diagnostic import START, apply, probe
from nps_threads import read_positions
from paired_search_diagnostics import freeze


def requests_for_row(position: str, row: dict):
    if position.startswith("sfen "):
        sfen = position[5:]
    elif position == "startpos":
        sfen = START
    else:
        raise ValueError("PV validation currently requires full SFEN or bare startpos")
    if not row["pv"] or row["pv"][0] != row["bestmove"]:
        raise ValueError("PV must begin with the reported legal bestmove")
    requests = []
    for ply, move in enumerate(row["pv"]):
        requests.append((sfen, move, ply))
        sfen = apply(sfen, move)
    return requests


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--legal-moves", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("runs", type=Path, nargs="+")
    args = parser.parse_args()
    frozen = args.output.with_suffix(".legal-moves")
    tool_hash = freeze(args.legal_moves, frozen)
    by_board = {}
    violations, runs = [], []
    rows_count = moves_count = 0
    for run in args.runs:
        manifest = json.loads((run / "manifest.json").read_text())
        if manifest["status"] != "complete":
            raise ValueError(f"cannot validate incomplete run {run}")
        positions = read_positions(str(run / "positions.input"), 1_000_000)
        runs.append({"path": str(run.resolve()), "engine_sha256": {side: manifest["engines"][side]["sha256"] for side in "AB"},
                     "corpus_sha256": manifest["positions_sha256"]})
        for line in (run / "rows.jsonl").read_text().splitlines():
            row = json.loads(line)
            rows_count += 1
            try:
                requests = requests_for_row(positions[row["position"]], row)
                for board, move, ply in requests:
                    moves_count += 1
                    by_board.setdefault(board, []).append({"run": run.name, "sequence": row["sequence"], "ply": ply, "move": move})
            except (ValueError, IndexError, KeyError, TypeError) as error:
                violations.append({"run": run.name, "sequence": row["sequence"], "error": str(error)})
    boards = list(by_board)
    legal = probe(frozen, boards) if boards else []
    for board, line in zip(boards, legal, strict=True):
        allowed = set(line.split())
        violations.extend({**request, "error": "PV move absent from legal move set"}
                          for request in by_board[board] if request["move"] not in allowed)
    result = {"schema": "sekirei.paired-pv-validation.v1", "rows": rows_count, "pv_moves": moves_count,
              "unique_board_probes": len(boards), "violations": violations,
              "legal_tool_sha256": tool_hash, "legal_tool_frozen": str(frozen.resolve()), "runs": runs,
              "interpretation": "Same-rules legal-PV check; not an independent oracle or mate-score proof."}
    with args.output.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(json.dumps({"rows": rows_count, "pv_moves": moves_count, "violations": len(violations)}))
    return bool(violations)


if __name__ == "__main__":
    raise SystemExit(main())
