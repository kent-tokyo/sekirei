#!/usr/bin/env python3
"""Record deterministic observations for the fixed SP0 sequence contract."""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from validate_sequence_contract import DEFAULT_PATH, canonical_sha256, validate


def parse_observation(output: str) -> dict[str, object]:
    line = next(
        line.removeprefix("sequence_observation=")
        for line in output.splitlines()
        if line.startswith("sequence_observation=")
    )
    legal_moves, perft2, divide = line.split(";", 2)
    divide_key, divide_value = divide.split("=", 1)
    if divide_key != "perft2_divide":
        raise ValueError("sequence observation has an unexpected divide key")
    return {
        "legal_moves": int(legal_moves.removeprefix("legal_moves:")),
        "perft2": int(perft2.removeprefix("perft2:")),
        "perft2_divide": divide_value.split(",") if divide_value else [],
    }


def record(binary: Path, contract: Path) -> dict[str, object]:
    document = json.loads(contract.read_text(encoding="utf-8"))
    count = validate(contract)
    series = []
    for item in document["series"]:
        result = subprocess.run(
            [str(binary), "--check-sequence", item["sfen"], " ".join(item["sequence"])],
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode:
            raise RuntimeError(f"sequence preflight failed for {item['id']}: {result.stderr}")
        series.append({**item, "observation": parse_observation(result.stdout)})
    return {
        "schema": "sekirei.sequence-observations.v1",
        "version": 1,
        "contract_sha256": canonical_sha256(document),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "series_count": count,
        "series": series,
        "notes": "Correctness and provenance observations; not a performance or playing-strength gate.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--contract", type=Path, default=DEFAULT_PATH)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = record(args.binary.resolve(strict=True), args.contract.resolve(strict=True))
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(f"sequence observations recorded: {args.output};series={report['series_count']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
