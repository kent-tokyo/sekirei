#!/usr/bin/env python3
"""Merge sharded Sekirei teacher caches without hiding contract conflicts."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def merge(paths: list[Path]) -> tuple[list[dict], int]:
    by_sfen: dict[str, dict] = {}
    duplicate_count = 0
    contract: tuple[int, str] | None = None
    for path in paths:
        for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if not raw.strip():
                continue
            try:
                row = json.loads(raw)
            except json.JSONDecodeError as error:
                raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
            try:
                current_contract = (int(row["label_depth"]), str(row["teacher_identity"]))
                sfen = str(row["sfen"])
                score = int(row["score_cp"])
            except (KeyError, TypeError, ValueError) as error:
                raise ValueError(f"{path}:{line_number}: invalid cache row") from error
            if not sfen:
                raise ValueError(f"{path}:{line_number}: empty SFEN")
            if contract is None:
                contract = current_contract
            elif current_contract != contract:
                raise ValueError(
                    f"{path}:{line_number}: cache contract {current_contract!r} != {contract!r}"
                )
            previous = by_sfen.get(sfen)
            if previous is not None:
                duplicate_count += 1
                if int(previous["score_cp"]) != score:
                    raise ValueError(
                        f"{path}:{line_number}: conflicting duplicate score for {sfen!r}"
                    )
            by_sfen[sfen] = {
                "sfen": sfen,
                "label_depth": current_contract[0],
                "teacher_identity": current_contract[1],
                "score_cp": score,
            }
    if not by_sfen:
        raise ValueError("no cache rows found")
    return [by_sfen[sfen] for sfen in sorted(by_sfen)], duplicate_count


def write_atomic(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, separators=(",", ":"), ensure_ascii=False) + "\n")
        handle.flush()
    temporary.replace(path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("inputs", nargs="+", type=Path)
    args = parser.parse_args()
    try:
        rows, duplicates = merge(args.inputs)
        write_atomic(args.output, rows)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(f"merged {len(args.inputs)} caches: {len(rows)} unique rows, {duplicates} duplicates")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
