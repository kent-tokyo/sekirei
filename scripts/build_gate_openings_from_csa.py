#!/usr/bin/env python3
"""Build a provenance-preserving, deterministic gate-opening corpus from CSA.

The tool delegates CSA parsing and SFEN materialization to
``sekirei-history-replay --export-json``.  It selects one non-opening prefix
per game, records every source hash and selected ply, and rejects duplicate
SFENs.  The resulting corpus is an input to a future gate, not evidence of
engine strength on its own.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path


SCHEMA = "sekirei.gate-opening-corpus.v1"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def select_position(document: dict, source_hash: str) -> dict | None:
    """Select one replayable, non-opening position deterministically."""
    if document.get("schema") != "sekirei.csa-replay.v2":
        return None
    positions = document.get("positions")
    if not isinstance(positions, list):
        return None
    eligible = [
        position for position in positions
        if isinstance(position, dict)
        and isinstance(position.get("ply"), int)
        and 12 <= position["ply"] <= 80
        and isinstance(position.get("pre_move_sfen"), str)
        and position["pre_move_sfen"]
        and isinstance(position.get("history_before_usi"), list)
        and all(isinstance(move, str) and move for move in position["history_before_usi"])
    ]
    if not eligible:
        return None
    index = int(source_hash[:16], 16) % len(eligible)
    return eligible[index]


def export_replay(binary: Path, source: Path) -> dict:
    with tempfile.TemporaryDirectory(prefix="sekirei-gate-opening-") as directory:
        output = Path(directory) / "replay.json"
        completed = subprocess.run(
            [str(binary), "--export-json", str(source), str(output)],
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0 or not output.is_file():
            raise ValueError(completed.stderr.strip() or "CSA replay export failed")
        return json.loads(output.read_text(encoding="utf-8"))


def build(binary: Path, source_dir: Path, count: int) -> tuple[list[str], list[dict], list[dict]]:
    lines: list[str] = []
    selected: list[dict] = []
    rejected: list[dict] = []
    seen_sfens: set[str] = set()
    for source in sorted(source_dir.glob("*.csa")):
        if len(lines) == count:
            break
        source_hash = sha256(source)
        try:
            position = select_position(export_replay(binary, source), source_hash)
        except (OSError, ValueError, json.JSONDecodeError) as error:
            rejected.append({"path": str(source), "reason": str(error)})
            continue
        if position is None:
            rejected.append({"path": str(source), "reason": "no_replayable_nonopening_position"})
            continue
        sfen = position["pre_move_sfen"]
        if sfen in seen_sfens:
            rejected.append({"path": str(source), "reason": "duplicate_sfen"})
            continue
        seen_sfens.add(sfen)
        lines.append(sfen)
        selected.append({
            "path": str(source),
            "sha256": source_hash,
            "ply": position["ply"],
            "sfen": sfen,
            "history_before_usi": position["history_before_usi"],
        })
    if len(lines) != count:
        raise ValueError(f"selected {len(lines)}/{count} unique replayable positions")
    return lines, selected, rejected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--history-binary", type=Path, required=True)
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--count", type=int, default=200)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    if args.count <= 0:
        parser.error("count must be positive")
    if not args.history_binary.is_file() or not args.source_dir.is_dir():
        parser.error("history binary or CSA source directory is unavailable")
    try:
        lines, selected, rejected = build(args.history_binary, args.source_dir, args.count)
    except ValueError as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines) + "\n", encoding="utf-8")
    manifest = {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "history_binary": {"path": str(args.history_binary), "sha256": sha256(args.history_binary)},
        "source_dir": str(args.source_dir),
        "positions": len(selected),
        "openings_sha256": sha256(args.output),
        "selected": selected,
        "rejected": rejected,
    }
    args.manifest.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(selected)} unique replayable positions")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
