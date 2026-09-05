#!/usr/bin/env python3
"""Verify that a SharedMcts transcript and manifest contain the same counts."""
import argparse
import json
import sys
from pathlib import Path

from record_mcts_transcript import parse_transcript
from validate_release_manifest import validate


def verify(manifest_path: Path, transcript_path: Path) -> None:
    manifest = json.loads(manifest_path.read_text())
    errors = validate(manifest)
    if errors:
        raise ValueError("invalid release manifest: " + ", ".join(errors))
    diagnostic = manifest.get("mcts_diagnostic")
    if not isinstance(diagnostic, dict):
        raise ValueError("manifest has no mcts_diagnostic")
    counts = parse_transcript(transcript_path.read_text())
    for key, value in counts.items():
        if diagnostic.get(key) != value:
            raise ValueError(
                f"mcts_diagnostic.{key} disagrees: "
                f"manifest={diagnostic.get(key)!r}, transcript={value!r}"
            )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--transcript", type=Path, required=True)
    args = parser.parse_args()
    try:
        verify(args.manifest, args.transcript)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    print(f"MCTS diagnostic verified: {args.manifest} <- {args.transcript}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
