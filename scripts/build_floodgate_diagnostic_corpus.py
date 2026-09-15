#!/usr/bin/env python3
"""Extract a replay-verified Floodgate position corpus without searching.

The output is for diagnostic/tuning review only.  The played move is retained
as an observation, never treated as a correct-move label or strength target.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from analyze_analysis_record import analyze, csa_moves


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def text_sha256(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _select_records(records: list[dict], limit: int | None,
                    controls_per_game: int) -> list[tuple[dict, str]]:
    swings = [record for record in records if record.get("sign_reversal")]
    if limit is not None:
        swings = swings[:limit]
    selected = [(record, "swing") for record in swings]
    if controls_per_game <= 0:
        return selected
    controls = [
        record for record in records
        if not record.get("sign_reversal")
        and record.get("score_kind") != "mate"
        and record.get("actual_move_csa")
    ][:controls_per_game]
    selected.extend((record, "control") for record in controls)
    return selected


def extract(csa: Path, analysis: Path, threshold: int = 200, limit: int | None = None,
            controls_per_game: int = 0) -> list[dict]:
    report = analyze(csa, analysis, threshold)
    if report["alignment_errors"]:
        raise ValueError("cannot extract from an alignment-invalid game")
    moves = csa_moves(csa)
    selected = _select_records(report["records"], limit, controls_per_game)
    entries = []
    for record, entry_kind in selected:
        ply = record["ply"]
        entries.append({
            "source": {
                "game_id": report["game_id"],
                "csa": str(csa),
                "csa_sha256": sha256(csa),
                "analysis": str(analysis),
                "analysis_sha256": sha256(analysis),
                "ply": ply,
                "result": report["result"],
            },
            "position": {
                "sfen": record.get("sfen"),
                "sfen_sha256": text_sha256(record["sfen"]),
                "side_to_move": record.get("side_to_move"),
                "actual_move_observed": record.get("actual_move_csa"),
                "engine_move_observed": record.get("bestmove_csa"),
                "moves_before": moves[max(0, ply - 2):ply],
                "history_before": moves[:ply],
                "score_before_cp": record.get("previous_score_cp"),
                "score_after_cp": record.get("score_cp"),
                "score_delta_cp": record.get("score_delta_cp"),
            },
            "entry_kind": entry_kind,
            "label_policy": "observation_only_no_correct_move_label",
        })
    return entries


def build(csa_dir: Path, analysis_dir: Path, threshold: int, limit: int | None,
          controls_per_game: int = 0) -> dict:
    games = []
    invalid = []
    for csa in sorted(csa_dir.glob("*.csa")):
        analysis = analysis_dir / f"{csa.stem}.analysis.jsonl"
        if not analysis.is_file():
            continue
        try:
            games.extend(extract(csa, analysis, threshold, limit, controls_per_game))
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            invalid.append({"csa": str(csa), "analysis": str(analysis), "error": str(exc)})
    return {
        "schema": "sekirei.floodgate-diagnostic-corpus.v1",
        "diagnostic_only": True,
        "split": "diagnostic_tuning_only",
        "threshold_cp": threshold,
        "entries": games,
        "invalid_games": invalid,
        "strength_claim": "not_permitted",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csa-dir", type=Path, required=True)
    parser.add_argument("--analysis-dir", type=Path, required=True)
    parser.add_argument("--threshold-cp", type=int, default=200)
    parser.add_argument("--limit-per-game", type=int)
    parser.add_argument("--controls-per-game", type=int, default=0,
                        help="also include this many nonterminal control records per game")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.controls_per_game < 0:
        parser.error("--controls-per-game must be non-negative")
    document = build(args.csa_dir, args.analysis_dir, args.threshold_cp,
                     args.limit_per_game, args.controls_per_game)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(document['entries'])} diagnostic positions")
    return 0 if not document["invalid_games"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
