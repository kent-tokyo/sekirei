#!/usr/bin/env python3
"""Expand the speed smoke corpus with verified opposite-side positions."""

import argparse
import json
import subprocess
from pathlib import Path


def details(binary: Path, sfen: str) -> tuple[dict[str, int], list[str]]:
    output = subprocess.check_output([str(binary), "--check-sfen", sfen], text=True)
    line = next(line for line in output.splitlines() if line.startswith("sfen_details="))
    counts = {key: int(value) for key, value in (
        item.split(":", 1) for item in line.removeprefix("sfen_details=").split(";")
    )}
    moves = next(line for line in output.splitlines() if line.startswith("sfen_moves="))
    return counts, moves.removeprefix("sfen_moves=").split(",")


def expand(path: Path, binary: Path) -> int:
    document = json.loads(path.read_text(encoding="utf-8"))
    original = [
        case for case in document["cases"]
        if not case["id"].endswith("-opposite")
    ]
    additions = []
    for case in original:
        expected, moves = details(binary, case["sfen"])
        case["expected"] = expected | {"legal_moves_usi": moves}
        fields = case["sfen"].split(" ")
        fields[1] = "w" if fields[1] == "b" else "b"
        opposite = dict(case)
        opposite["id"] = f"{case['id']}-opposite"
        opposite["sfen"] = " ".join(fields)
        opposite["source"] = f"{case['source']}-opposite-side"
        expected, moves = details(binary, opposite["sfen"])
        opposite["expected"] = expected | {"legal_moves_usi": moves}
        additions.append(opposite)
    document["purpose"] = (
        "SP0 64-position smoke corpus: 8 categories x 4 base positions x 2 sides; "
        "legal-move and cross-library preflight is required before timing"
    )
    document["cases"] = original + additions
    document["required_case_ids"] = [case["id"] for case in document["cases"]]
    path.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return len(document["cases"])


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    args = parser.parse_args()
    count = expand(args.corpus, args.binary.resolve(strict=True))
    print(f"expanded speed corpus: cases={count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
