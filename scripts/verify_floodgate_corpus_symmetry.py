#!/usr/bin/env python3
"""Run a small legality smoke on each corpus SFEN and its 180-degree image."""
from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

from sfen_symmetry import rotate_sfen


def parse_diagnostic(output: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for field in output.strip().split("\t"):
        if "=" not in field:
            continue
        key, value = field.split("=", 1)
        values[key] = value
    return values


def verify(binary: Path, corpus: Path, nodes: int = 64) -> list[str]:
    document = json.loads(corpus.read_text(encoding="utf-8"))
    failures: list[str] = []
    for index, entry in enumerate(document.get("entries", [])):
        sfen = entry["position"]["sfen"]
        for variant, candidate in (("original", sfen), ("rotated", rotate_sfen(sfen))):
            result = subprocess.run(
                [str(binary), "--nodes", str(nodes), "--sfen", candidate],
                capture_output=True, text=True, check=False,
            )
            if result.returncode != 0:
                failures.append(f"entries[{index}] {variant}: exit {result.returncode}")
                continue
            values = parse_diagnostic(result.stdout)
            if values.get("pv_legal") != "true":
                failures.append(f"entries[{index}] {variant}: pv_legal={values.get('pv_legal')}")
            if values.get("pv_replay_preserves_input") != "true":
                failures.append(
                    f"entries[{index}] {variant}: pv_replay_preserves_input="
                    f"{values.get('pv_replay_preserves_input')}"
                )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=64)
    args = parser.parse_args()
    if args.nodes <= 0:
        parser.error("--nodes must be positive")
    failures = verify(args.binary, args.corpus, args.nodes)
    if failures:
        print("symmetry legality verification failed: " + "; ".join(failures))
        return 1
    print(f"symmetry legality verification passed: {args.corpus}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
