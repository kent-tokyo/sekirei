#!/usr/bin/env python3
"""Create an MCTS diagnostic manifest copy from a USI transcript."""
import argparse
import json
import re
from pathlib import Path

from record_mcts_manifest import record


DIAGNOSTIC = re.compile(
    r"^(?:<\s*)?info string shared_mcts simulations (?P<simulations>\d+) "
    r"arena_nodes (?P<arena_nodes>\d+) "
    r"transposition_hits (?P<transposition_hits>\d+)\s*$",
    re.MULTILINE,
)


def parse_transcript(transcript: str) -> dict[str, int]:
    matches = list(DIAGNOSTIC.finditer(transcript))
    if not matches:
        raise ValueError("transcript has no shared_mcts diagnostic line")
    match = matches[-1]
    return {key: int(value) for key, value in match.groupdict().items()}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--release-manifest", type=Path, required=True)
    parser.add_argument("--transcript", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        counts = parse_transcript(args.transcript.read_text())
        record(args.release_manifest, args.output, "SharedMcts", **counts)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    print(f"MCTS transcript manifest copy written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
