#!/usr/bin/env python3
"""Freeze three new CSA positions using a registered ply floor, without search."""

import argparse
import hashlib
import json
from pathlib import Path

RESULTS_SHA = "fba23cb84d664e21eb09e70494929d253ce6874890d0e686e0329414ecf75998"
RECEIPT_SHA = "8edfa88ab0263d5544f50f7ce106e560bf95a975ff1c6b8fb76b4e72a56cf551"
S18_CORPUS_SHA = "f046b3a6c4fdfee3350648e0087e6380b9623045d5f2469bf6b86a3928eb7ed9"
CLOCK_TRACE_SHA = "985361c8a04cf40274964931320e91c0fe2c536218eae5220c971cf951d843c5"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checked_json(path: Path, expected: str):
    assert digest(path) == expected, f"SHA mismatch: {path}"
    return json.loads(path.read_text())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trial", type=Path)
    parser.add_argument("s17", type=Path)
    parser.add_argument("clock_trace", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--min-ply", type=int, required=True)
    args = parser.parse_args()

    results = checked_json(args.trial / "results-verified.json", RESULTS_SHA)
    receipt = checked_json(args.s17 / "replay_receipt.json", RECEIPT_SHA)
    s18 = checked_json(args.s17 / "corpus.json", S18_CORPUS_SHA)
    assert digest(args.clock_trace) == CLOCK_TRACE_SHA
    lines = [json.loads(line) for line in args.clock_trace.read_text().splitlines()]
    assert lines[0]["source_results_sha256"] == RESULTS_SHA
    previous = {(entry["source"]["game_id"], entry["source"]["ply"])
                for entry in s18["entries"]}

    entries = []
    for number in (6, 7, 8):
        game = next(game for game in results["games"] if game["game"] == number)
        replay_receipt = next(row for row in receipt["games"] if row["game"] == number)
        assert replay_receipt["csa_sha256"] == game["public_sha256"]
        source_csa = args.trial / "manifests" / f"public-game-{number:02}.csa"
        assert digest(source_csa) == game["public_sha256"]
        replay = checked_json(args.s17 / "replay" / f"game-{number:02}.json",
                              replay_receipt["replay_sha256"])
        assert replay["terminal_complete"] and replay["player"] == "sekirei-v051-r6"
        assert len(replay["positions"]) == replay_receipt["positions"]
        own_positions = [position for position in replay["positions"]
                         if position["is_player_to_move"]]
        clock = [row for row in lines[1:] if row["game"] == number]
        assert len(own_positions) == len(clock)
        selected = None
        for own_move, (position, row) in enumerate(zip(own_positions, clock), start=1):
            assert row["own_move"] == own_move
            assert len(position["history_before_usi"]) == position["ply"]
            assert position["side_to_move"] == replay["player_color"]
            if (position["ply"] >= args.min_ply
                    and row["candidate_budget_ms"] - row["legacy_budget_ms"] >= 500
                    and (replay["game_id"], position["ply"]) not in previous):
                selected = (position, row)
                break
        assert selected is not None, f"no position at ply >= {args.min_ply} in game {number}"
        position, row = selected
        assert position["pre_move_sfen"] not in {
            entry["position"]["sfen"] for entry in s18["entries"]
        }, f"S18 SFEN overlap in game {number}"
        entries.append({
            "game": number,
            "public_url": game["public_url"],
            "public_csa_sha256": game["public_sha256"],
            "replay_sha256": replay_receipt["replay_sha256"],
            "game_id": replay["game_id"],
            "ply": position["ply"],
            "own_move": row["own_move"],
            "bank_before_ms": row["bank_before_ms"],
            "legacy_budget_ms": row["legacy_budget_ms"],
            "candidate_budget_ms": row["candidate_budget_ms"],
            "initial_sfen": replay["initial_sfen"],
            "pre_move_sfen": position["pre_move_sfen"],
            "history_before_usi": position["history_before_usi"],
            "observed_move_is_not_a_quality_label": True,
        })

    payload = {"schema": "sekirei.issue283.clock-inputs.v1",
               "selection": f"first own ply >={args.min_ply} with candidate-minus-legacy >=500ms per game 6,7,8",
               "source_results_sha256": RESULTS_SHA,
               "s17_receipt_sha256": RECEIPT_SHA,
               "s18_corpus_sha256": S18_CORPUS_SHA,
               "clock_trace_sha256": CLOCK_TRACE_SHA,
               "entries": entries}
    assert len({entry["pre_move_sfen"] for entry in entries}) == 3
    args.output.write_text(json.dumps(payload, indent=2) + "\n")
    print(json.dumps([{"game": row["game"], "ply": row["ply"],
                       "legacy_ms": row["legacy_budget_ms"],
                       "candidate_ms": row["candidate_budget_ms"]} for row in entries]))


if __name__ == "__main__":
    main()
