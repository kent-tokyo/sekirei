#!/usr/bin/env python3
"""Freeze legally replayed Lishogi positions without treating solutions as proof."""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

STARTPOS = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replay(helper: Path, sfen: str, moves: list[str]) -> str:
    output = subprocess.check_output(
        [str(helper), "--sfen", sfen, "--moves", " ".join(moves)],
        text=True,
        timeout=5,
    )
    return dict(field.split("=", 1) for field in output.strip().split("\t"))["sfen"]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--history-sfen", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    manifest_path = args.source_dir / "collection_manifest.json"
    manifest = json.loads(manifest_path.read_text())
    cases = []
    seen_games = set()
    seen_positions = set()
    exclusions = []
    for response in manifest["responses"]:
        puzzle_id = response.get("id")
        if response.get("http_status") != 200 or response.get("selection") != "selected":
            continue
        path = args.source_dir / f"{puzzle_id}.json"
        document = json.loads(path.read_text())["data"]["data"]
        puzzle = document["puzzle"]
        game = document["game"]
        if puzzle["id"] != puzzle_id or game["id"] != response["game_id"]:
            raise ValueError(f"source identity mismatch for {puzzle_id}")
        raw_html = args.source_dir / Path(response["path"]).name
        if sha256(raw_html) != response["html_sha256"]:
            raise ValueError(f"raw HTML hash mismatch for {puzzle_id}")
        themes = puzzle["themes"]
        if any("mate" in theme.lower() or "tsume" in theme.lower() for theme in themes):
            exclusions.append({"id": puzzle_id, "reason": "mate_theme"})
            continue
        if game["id"] in seen_games:
            exclusions.append({"id": puzzle_id, "reason": "repeated_game"})
            continue
        moves = game["moves"].split()
        if len(moves) != puzzle["initialPly"] + 1 or moves[-1] != puzzle["initialUsi"]:
            raise ValueError(f"game/initial move mismatch for {puzzle_id}")
        if replay(args.history_sfen, STARTPOS, moves[:-1]) != puzzle["initialSfen"]:
            raise ValueError(f"full game replay differs from initial SFEN for {puzzle_id}")
        position = replay(args.history_sfen, puzzle["initialSfen"], [puzzle["initialUsi"]])
        if position in seen_positions:
            exclusions.append({"id": puzzle_id, "reason": "repeated_position"})
            continue
        if not puzzle["solution"]:
            raise ValueError(f"missing solution for {puzzle_id}")
        replay(args.history_sfen, position, [puzzle["solution"][0]])
        seen_games.add(game["id"])
        seen_positions.add(position)
        cases.append({
            "id": puzzle_id,
            "game_id": game["id"],
            "source_url": response["source_url"],
            "canonical_url": response["canonical_url"],
            "html_sha256": response["html_sha256"],
            "source_json_sha256": sha256(path),
            "themes": themes,
            "initial_sfen": puzzle["initialSfen"],
            "initial_usi": puzzle["initialUsi"],
            "sfen": position,
            "reference_first_move": puzzle["solution"][0],
            "reference_policy": "metadata_only_not_a_correctness_oracle",
        })
        if len(cases) == 24:
            break
    split = (len(cases) + 1) // 2
    for index, case in enumerate(cases):
        case["split"] = "development" if index < split else "confirmation"
    result = {
        "schema": "sekirei.public-next-move.v1",
        "source_manifest_sha256": sha256(manifest_path),
        "history_sfen_sha256": sha256(args.history_sfen),
        "cases": cases,
        "exclusions": exclusions,
        "count": len(cases),
        "count_target": 24,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"count": len(cases), "development": split,
                      "confirmation": len(cases) - split, "output": str(args.output)}))


if __name__ == "__main__":
    main()
