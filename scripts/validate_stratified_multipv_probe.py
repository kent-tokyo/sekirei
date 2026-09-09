#!/usr/bin/env python3
"""Validate a stratified MultiPV diagnostic report without running an engine."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def validate(report: dict) -> list[str]:
    errors = []
    if report.get("schema_version") != 1:
        errors.append("schema_version must be 1")
    if report.get("diagnostic_only") is not True:
        errors.append("diagnostic_only must be true")
    positions = report.get("positions")
    if not isinstance(positions, list):
        return errors + ["positions must be an array"]
    seen = set()
    for index, position in enumerate(positions):
        sample_id = position.get("sample_id")
        if not isinstance(sample_id, str) or not sample_id:
            errors.append(f"positions[{index}].sample_id is missing")
        elif sample_id in seen:
            errors.append(f"duplicate sample_id: {sample_id}")
        seen.add(sample_id)
        for name in ("candidate", "teacher", "material"):
            evaluator = position.get("evaluators", {}).get(name, {})
            if evaluator.get("status") != "ok":
                continue
            lines = evaluator.get("lines")
            if not isinstance(lines, list) or not lines:
                errors.append(f"positions[{index}].{name}.lines is empty")
                continue
            multipv = [line.get("multipv") for line in lines]
            if multipv != sorted(set(multipv)):
                errors.append(f"positions[{index}].{name}.multipv is not unique and ordered")
            if any(not isinstance(line.get("move"), str) or not line["move"] for line in lines):
                errors.append(f"positions[{index}].{name} has an invalid move")
            if any(not isinstance(line.get("score_cp"), int) for line in lines):
                errors.append(f"positions[{index}].{name} has a non-integer score")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    errors = validate(json.loads(args.report.read_text(encoding="utf-8")))
    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        return 1
    print(json.dumps({"valid": True, "positions": len(json.loads(args.report.read_text(encoding="utf-8"))["positions"])}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
