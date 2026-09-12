#!/usr/bin/env python3
"""Run the Rust cross-library correctness preflight for the smoke corpus."""

import argparse
import subprocess
from pathlib import Path

from validate_speed_corpus import DEFAULT_CORPUS, validate


def run_case(binary: Path, sfen: str, runner=subprocess.run) -> str:
    result = runner(
        [str(binary), "--check-sfen", sfen],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        raise RuntimeError(
            f"SFEN preflight failed ({result.returncode}): {sfen}\n"
            f"stdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout


def preflight(binary: Path, corpus: Path = DEFAULT_CORPUS, runner=subprocess.run) -> int:
    import json

    document = json.loads(corpus.read_text(encoding="utf-8"))
    count = validate(corpus)
    for case in document["cases"]:
        output = run_case(binary, case["sfen"], runner)
        details = next(
            (line.removeprefix("sfen_details=") for line in output.splitlines()
             if line.startswith("sfen_details=")),
            None,
        )
        if details is None:
            raise RuntimeError(f"SFEN preflight omitted numeric details: {case['id']}")
        observed = dict(item.split(":", 1) for item in details.split(";"))
        expected = case["expected"]
        if observed.get("legal_moves") != str(expected["legal_moves"]):
            raise RuntimeError(f"legal move count mismatch: {case['id']}")
        if observed.get("perft2") != str(expected["perft2"]):
            raise RuntimeError(f"Perft(2) mismatch: {case['id']}")
    return count


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    args = parser.parse_args()
    try:
        count = preflight(args.binary.resolve(strict=True), args.corpus.resolve(strict=True))
    except (OSError, RuntimeError, ValueError) as error:
        parser.error(str(error))
    print(f"speed corpus preflight OK: cases={count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
