#!/usr/bin/env python3
"""Run the Rust do/undo preflight for the fixed SP0 sequence contract."""

import argparse
import json
import subprocess
from pathlib import Path

from validate_sequence_contract import DEFAULT_PATH, canonical_sha256, validate


def preflight(binary: Path, contract: Path = DEFAULT_PATH) -> tuple[int, str]:
    document = json.loads(contract.read_text(encoding="utf-8"))
    count = validate(contract)
    for item in document["series"]:
        result = subprocess.run(
            [str(binary), "--check-sequence", item["sfen"], " ".join(item["sequence"])],
            check=False,
            capture_output=True,
            text=True,
        )
        marker = "sequence_preflight=passed;"
        observation_marker = "sequence_observation=legal_moves:"
        if result.returncode or marker not in result.stdout or observation_marker not in result.stdout:
            raise RuntimeError(
                f"sequence preflight failed for {item['id']}: "
                f"stdout={result.stdout} stderr={result.stderr}"
            )
    return count, canonical_sha256(document)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--contract", type=Path, default=DEFAULT_PATH)
    args = parser.parse_args()
    try:
        count, digest = preflight(args.binary.resolve(strict=True), args.contract.resolve(strict=True))
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(f"sequence contract preflight OK: series={count};contract_sha256={digest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
