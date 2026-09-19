#!/usr/bin/env python3
"""Compare exact legal USI move sets with cshogi 1.0.5 in a Linux container.

The cshogi process is an optional development oracle. It is isolated from
Sekirei's Rust workspace, runs a digest-pinned Python image, and receives only
the newline-delimited SFEN corpus via stdin. A disagreement is diagnostic
evidence, not an automatic declaration that either implementation is wrong.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_INPUT = ROOT / "scripts" / "fixtures" / "cshogi_boundary_cases_v1.json"
DEFAULT_SEKIREI = ROOT / "target" / "release" / "sekirei-legal-moves"
CSHOGI_PROGRAM = (
    "import sys,cshogi; "
    "[print(' '.join(sorted(cshogi.move_to_usi(m) "
    "for m in cshogi.Board(line.strip()).legal_moves))) for line in sys.stdin if line.strip()]"
)


def run(command: list[str], stdin: str) -> list[list[str]]:
    completed = subprocess.run(command, input=stdin, text=True, capture_output=True)
    if completed.returncode:
        raise SystemExit(completed.stderr.strip() or f"oracle command failed: {command[0]}")
    return [line.split() for line in completed.stdout.splitlines()]


def load_cases(path: Path) -> list[dict[str, object]]:
    """Load the versioned boundary schema, retaining legacy SFEN-list input.

    The JSON form is the supported contract: every case records why it exists
    and an expected legal set or count independent of the optional cshogi
    comparison.  A plain line-delimited SFEN file remains useful for an
    ad-hoc corpus, but cannot by itself serve as evidence for a named rule.
    """
    text = path.read_text(encoding="utf-8")
    if path.suffix != ".json":
        cases = [
            {"id": f"legacy-{index}", "initial_sfen": line.strip()}
            for index, line in enumerate(text.splitlines())
            if line.strip()
        ]
        if not cases:
            raise ValueError("input corpus is empty")
        return cases

    document = json.loads(text)
    if document.get("schema") != "sekirei.cshogi-boundary-cases.v1":
        raise ValueError("unsupported boundary corpus schema")
    raw_cases = document.get("cases")
    if not isinstance(raw_cases, list) or not raw_cases:
        raise ValueError("boundary corpus has no cases")
    cases: list[dict[str, object]] = []
    identifiers: set[str] = set()
    for case in raw_cases:
        if not isinstance(case, dict):
            raise ValueError("boundary case must be an object")
        identifier = case.get("id")
        sfen = case.get("initial_sfen")
        history = case.get("history_usi")
        expected_moves = case.get("expected_legal_usi")
        expected_count = case.get("expected_legal_count")
        if not isinstance(identifier, str) or not identifier or identifier in identifiers:
            raise ValueError("boundary case id must be unique")
        if not isinstance(sfen, str) or not sfen:
            raise ValueError(f"{identifier}: missing initial_sfen")
        if not isinstance(history, list) or not all(isinstance(move, str) and move for move in history):
            raise ValueError(f"{identifier}: invalid history_usi")
        if expected_moves is None and expected_count is None:
            raise ValueError(f"{identifier}: missing expected legal move contract")
        if expected_moves is not None:
            if (not isinstance(expected_moves, list)
                    or not all(isinstance(move, str) and move for move in expected_moves)
                    or expected_moves != sorted(set(expected_moves))):
                raise ValueError(f"{identifier}: expected_legal_usi must be sorted and unique")
        if expected_count is not None and (not isinstance(expected_count, int) or expected_count < 0):
            raise ValueError(f"{identifier}: invalid expected_legal_count")
        if isinstance(expected_moves, list) and isinstance(expected_count, int) and len(expected_moves) != expected_count:
            raise ValueError(f"{identifier}: expected legal set/count disagree")
        identifiers.add(identifier)
        cases.append(case)
    return cases


def expected_matches(case: dict[str, object], moves: list[str]) -> bool:
    expected_moves = case.get("expected_legal_usi")
    if isinstance(expected_moves, list) and moves != expected_moves:
        return False
    expected_count = case.get("expected_legal_count")
    return not isinstance(expected_count, int) or len(moves) == expected_count


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, default=DEFAULT_INPUT)
    parser.add_argument("--sekirei", type=Path, default=DEFAULT_SEKIREI)
    parser.add_argument("--image", default="sekirei-cshogi-oracle:1.0.5")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    try:
        cases = load_cases(args.input)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(str(error)) from error
    sfens = [str(case["initial_sfen"]) for case in cases]
    if not args.sekirei.is_file():
        raise SystemExit(f"build release oracle first: {args.sekirei}")
    stdin = "\n".join(sfens) + "\n"
    sekirei = run([str(args.sekirei)], stdin)
    cshogi = run(["docker", "run", "--rm", "-i", args.image, "python", "-c", CSHOGI_PROGRAM], stdin)
    if len(sekirei) != len(sfens) or len(cshogi) != len(sfens):
        raise SystemExit("oracle line count does not match input corpus")
    rows = [
        {
            "id": case["id"],
            "sfen": sfen,
            "source": case.get("source"),
            "rule": case.get("rule"),
            "history_usi": case.get("history_usi"),
            "sekirei_moves": moves,
            "cshogi_moves": oracle_moves,
            "expected_matches": expected_matches(case, moves),
            "oracle_matches": moves == oracle_moves,
            "matches": expected_matches(case, moves) and moves == oracle_moves,
        }
        for case, sfen, moves, oracle_moves in zip(cases, sfens, sekirei, cshogi, strict=True)
    ]
    report = {
        "oracle": "cshogi==1.0.5",
        "image": args.image,
        "positions": len(rows),
        "matches": all(row["matches"] for row in rows),
        "rows": rows,
    }
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")
    if not report["matches"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
