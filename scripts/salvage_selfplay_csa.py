#!/usr/bin/env python3
"""Verify interrupted self-play CSA files and write a derived training manifest.

The input directory is never changed.  Each accepted game is replayed through
``sekirei-history-replay`` so this tool does not maintain a second move parser.
Missing game numbers remain observations of an interrupted run, not invented
MaxMoves or unplayed games.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from collections import Counter
from pathlib import Path


SCHEMA = "sekirei.selfplay-csa-salvage.v1"
TERMINAL_RESULTS = {"win", "lose", "draw", "resign", "jishogi", "repetition"}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def game_number(path: Path) -> int:
    stem = path.stem
    if not stem.startswith("game") or not stem[4:].isdigit():
        raise ValueError(f"not a gameNNNN CSA filename: {path.name}")
    return int(stem[4:])


def canonical_initial_sfen(sfen: str) -> str:
    fields = sfen.split()
    if len(fields) < 3:
        raise ValueError(f"invalid initial SFEN: {sfen!r}")
    # Move number is not part of a position family.  Board, side, and hands
    # are; retaining all three prevents colour- or hand-variant leakage.
    return " ".join(fields[:3])


def split_name(canonical_sfen: str) -> str:
    # Grouped deterministic split: all colour swaps and exact duplicate games
    # from one opening must remain on one side of validation.
    return "validation" if int(hashlib.sha256(canonical_sfen.encode()).hexdigest(), 16) % 5 == 0 else "train"


def export_game(history_replay: Path, source: Path, output: Path) -> dict:
    result = subprocess.run(
        [str(history_replay), "--export-json", str(source), str(output)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise ValueError(result.stderr.strip() or result.stdout.strip() or "history replay failed")
    return json.loads(output.read_text(encoding="utf-8"))


def salvage(csa_dir: Path, output_dir: Path, history_replay: Path, scheduled_games: int | None) -> dict:
    files = sorted(csa_dir.glob("game*.csa"), key=game_number)
    if not files:
        raise ValueError(f"no gameNNNN.csa files in {csa_dir}")
    if not history_replay.is_file():
        raise ValueError(f"history replay binary is missing: {history_replay}")
    output_dir.mkdir(parents=True, exist_ok=True)
    games_dir = output_dir / "games"
    games_dir.mkdir(parents=True, exist_ok=True)

    games = []
    invalid = []
    for source in files:
        number = game_number(source)
        derived = games_dir / f"{source.stem}.json"
        try:
            document = export_game(history_replay, source, derived)
            result = document.get("result")
            initial_sfen = document.get("initial_sfen")
            positions = document.get("positions")
            if result not in TERMINAL_RESULTS or document.get("terminal_complete") is not True:
                raise ValueError(f"non-terminal or unsupported result: {result!r}")
            if not isinstance(initial_sfen, str) or not isinstance(positions, list):
                raise ValueError("history replay omitted initial SFEN or positions")
            canonical = canonical_initial_sfen(initial_sfen)
            moves = [row.get("actual_move_csa") for row in positions]
            if any(not isinstance(move, str) for move in moves):
                raise ValueError("history replay omitted a CSA move")
            games.append({
                "game_number": number,
                "source": {"path": str(source), "sha256": sha256(source), "bytes": source.stat().st_size},
                "replay": {"path": str(derived), "sha256": sha256(derived), "positions": len(positions)},
                "terminal_result": result,
                "initial_sfen": initial_sfen,
                "canonical_initial_sfen": canonical,
                "split": split_name(canonical),
                "move_sequence_sha256": hashlib.sha256("\\n".join(moves).encode()).hexdigest(),
                "usable_for_cp_teacher": True,
                "observed_score_cp": "missing",
            })
        except ValueError as error:
            invalid.append({"game_number": number, "source": str(source), "error": str(error)})

    # A self-play pair can deterministically reproduce an identical game.  It
    # is useful evidence that the run happened, but counting its positions a
    # second time would overweight one opening in the teacher corpus.  Retain
    # every source record and select one representative per exact game.
    representatives: dict[tuple[str, str], int] = {}
    for game in games:
        identity = (game["canonical_initial_sfen"], game["move_sequence_sha256"])
        first = representatives.get(identity)
        if first is None:
            representatives[identity] = game["game_number"]
            game["dedup_status"] = "representative"
            game["usable_for_cp_teacher"] = True
        else:
            game["dedup_status"] = "exact_duplicate"
            game["duplicate_of_game_number"] = first
            game["usable_for_cp_teacher"] = False

    observed_numbers = {game["game_number"] for game in games} | {row["game_number"] for row in invalid}
    missing_numbers = (
        [number for number in range(1, scheduled_games + 1) if number not in observed_numbers]
        if scheduled_games is not None else []
    )
    duplicate_sequences = Counter((game["canonical_initial_sfen"], game["move_sequence_sha256"]) for game in games)
    terminal_counts = Counter(game["terminal_result"] for game in games)
    split_counts = Counter(game["split"] for game in games if game["usable_for_cp_teacher"])
    raw_split_counts = Counter(game["split"] for game in games)
    groups = {game["canonical_initial_sfen"] for game in games}
    document = {
        "schema": SCHEMA,
        "status": "recovered" if not invalid else "recovered_with_invalid_games",
        "source_run": str(csa_dir.parent),
        "source_csa_directory": str(csa_dir),
        "source_manifest": "missing",
        "scheduled_games": scheduled_games,
        "observed_csa_files": len(files),
        "replayed_games": len(games),
        "invalid_games": invalid,
        "missing_game_numbers": missing_numbers,
        "missing_game_number_interpretation": "unknown: may be unplayed, MaxMoves-excluded, or interrupted",
        "terminal_counts": dict(sorted(terminal_counts.items())),
        "unique_initial_position_groups": len(groups),
        "split_counts": dict(sorted(split_counts.items())),
        "raw_split_counts": dict(sorted(raw_split_counts.items())),
        "teacher_eligible_games": sum(game["usable_for_cp_teacher"] for game in games),
        "exact_duplicate_games": sum(count - 1 for count in duplicate_sequences.values() if count > 1),
        "games": games,
        "strength_claim": False,
        "training_eligibility": (
            "requires fixed teacher, independent split ledger, and teacher-cache provenance; "
            "this manifest alone is not a strength result"
        ),
    }
    manifest_path = output_dir / "manifest.json"
    manifest_path.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csa-dir", type=Path, required=True)
    parser.add_argument("--history-replay", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--scheduled-games", type=int)
    args = parser.parse_args()
    if args.scheduled_games is not None and args.scheduled_games <= 0:
        parser.error("--scheduled-games must be positive")
    try:
        report = salvage(args.csa_dir, args.output_dir, args.history_replay, args.scheduled_games)
    except ValueError as error:
        parser.error(str(error))
    print(json.dumps({
        "status": report["status"], "replayed_games": report["replayed_games"],
        "invalid_games": len(report["invalid_games"]),
        "unique_initial_position_groups": report["unique_initial_position_groups"],
        "split_counts": report["split_counts"],
    }, ensure_ascii=False))
    return 0 if not report["invalid_games"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
