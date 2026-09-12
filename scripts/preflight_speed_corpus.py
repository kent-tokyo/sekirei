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


def run_sequence(binary: Path, sfen: str, sequence: list[str], runner=subprocess.run) -> str:
    result = runner(
        [str(binary), "--check-sequence", sfen, " ".join(sequence)],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        raise RuntimeError(
            f"sequence preflight failed ({result.returncode}): {sfen} {sequence}\n"
            f"stdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout


def run_generated_sequence(binary: Path, sfen: str, plies: int, runner=subprocess.run) -> str:
    result = runner(
        [str(binary), "--check-generated-sequence", sfen, str(plies)],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        raise RuntimeError(
            f"generated sequence preflight failed ({result.returncode}): {sfen}\n"
            f"stdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout


def preflight(binary: Path, corpus: Path = DEFAULT_CORPUS, runner=subprocess.run) -> int:
    import json

    document = json.loads(corpus.read_text(encoding="utf-8"))
    count = validate(corpus)
    for case in document["cases"]:
        output = run_case(binary, case["sfen"], runner)
        sequence_output = run_sequence(binary, case["sfen"], case["sequence"], runner)
        if not any(line.startswith("sequence_preflight=passed;") for line in sequence_output.splitlines()):
            raise RuntimeError(f"sequence preflight omitted success marker: {case['id']}")
        generated_output = run_generated_sequence(binary, case["sfen"], 12, runner)
        if not any(line.startswith("generated_sequence_preflight=passed;") for line in generated_output.splitlines()):
            raise RuntimeError(f"generated sequence preflight omitted success marker: {case['id']}")
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
        moves_line = next(
            (line.removeprefix("sfen_moves=") for line in output.splitlines()
             if line.startswith("sfen_moves=")),
            None,
        )
        if moves_line is None:
            raise RuntimeError(f"SFEN preflight omitted legal move set: {case['id']}")
        observed_moves = moves_line.split(",") if moves_line else []
        if observed_moves != expected["legal_moves_usi"]:
            raise RuntimeError(f"legal move set mismatch: {case['id']}")
        divide_line = next(
            (line.removeprefix("sfen_perft2_divide=") for line in output.splitlines()
             if line.startswith("sfen_perft2_divide=")),
            None,
        )
        if divide_line is None:
            raise RuntimeError(f"SFEN preflight omitted Perft(2) divide: {case['id']}")
        if divide_line.split(",") != expected["perft2_divide"]:
            raise RuntimeError(f"Perft(2) divide mismatch: {case['id']}")
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
