#!/usr/bin/env python3
"""Validate a small engine-match calibration result before a strength gate."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


REQUIRED = ("games", "engine1_wins", "draws", "engine2_wins", "engine1_score", "diversity_ratio")


def validate(data: dict[str, object], expected_games: int | None = None) -> list[str]:
    errors: list[str] = []
    for key in REQUIRED:
        if key not in data:
            errors.append(f"missing {key}")
    if errors:
        return errors
    games = int(data["games"])
    if expected_games is not None and games != expected_games:
        errors.append(f"games={games}, expected {expected_games}")
    counts = sum(int(data[key]) for key in ("engine1_wins", "draws", "engine2_wins"))
    if counts != games:
        errors.append(f"outcome counts={counts}, games={games}")
    score = float(data["engine1_score"])
    if not 0.0 <= score <= 1.0:
        errors.append("engine1_score outside [0,1]")
    diversity = float(data["diversity_ratio"])
    if not 0.0 <= diversity <= 1.0:
        errors.append("diversity_ratio outside [0,1]")
    if int(data.get("unique_prefix20", 0)) < games:
        errors.append("unique_prefix20 is below games; calibration is not fully diverse")
    if int(data.get("top_prefix20_count", 0)) > 1:
        errors.append("top_prefix20_count indicates a repeated opening")
    return errors


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path)
    parser.add_argument("--expected-games", type=int)
    args = parser.parse_args()
    data = json.loads(args.result.read_text(encoding="utf-8"))
    errors = validate(data, args.expected_games)
    if errors:
        raise SystemExit("invalid calibration: " + "; ".join(errors))
    print(f"calibration valid: {args.result} ({data['games']} games)")


if __name__ == "__main__":
    main()
