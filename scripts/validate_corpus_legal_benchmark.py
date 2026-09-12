#!/usr/bin/env python3
"""Validate the 128-position corpus legal-generation benchmark output."""

import csv
import re
import sys
from io import StringIO
from pathlib import Path

SCHEMA = "sekirei.corpus-legal-benchmark.v1"
LIBRARIES = {"sekirei_generate_vec", "rsshogi_generate_move32"}
SCOPE = "corpus_sfen_setup_excluded_reused_buffer"
HEADER = "operation,library,median_ns_per_iteration,min_ns_per_iteration,max_ns_per_iteration,comparison_scope"
NUMBER = re.compile(r"^[0-9]+(?:\.[0-9]+)?$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")


def validate_text(text: str) -> int:
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if len(lines) < 5 or lines[0] != f"schema={SCHEMA}":
        raise ValueError("unexpected corpus legal benchmark schema")
    metadata = dict(item.split("=", 1) for item in lines[1].split(","))
    if metadata.get("cases") != "128":
        raise ValueError("cases must be 128")
    if not metadata.get("iterations", "").isdigit() or int(metadata["iterations"]) < 1:
        raise ValueError("iterations must be a positive integer")
    if metadata.get("samples") != "7":
        raise ValueError("samples must be 7")
    if not lines[2].startswith("corpus_sha256=") or not SHA256.fullmatch(lines[2].split("=", 1)[1]):
        raise ValueError("corpus_sha256 must be a SHA-256 hex digest")
    if lines[3] != HEADER:
        raise ValueError("unexpected CSV header")
    rows = list(csv.DictReader(StringIO("\n".join(lines[3:]))))
    if len(rows) != 256:
        raise ValueError("benchmark must contain exactly 256 rows")
    grouped = {}
    for row in rows:
        if set(row) != set(HEADER.split(",")):
            raise ValueError("unexpected benchmark columns")
        operation, library = row["operation"], row["library"]
        if not operation or library not in LIBRARIES:
            raise ValueError("invalid operation or library")
        if row["comparison_scope"] != SCOPE:
            raise ValueError("unexpected comparison scope")
        timing = {}
        for key in ("median_ns_per_iteration", "min_ns_per_iteration", "max_ns_per_iteration"):
            if not NUMBER.fullmatch(row[key]) or float(row[key]) <= 0:
                raise ValueError(f"invalid timing for {operation}/{library}")
            timing[key] = float(row[key])
        if not timing["min_ns_per_iteration"] <= timing["median_ns_per_iteration"] <= timing["max_ns_per_iteration"]:
            raise ValueError(f"timing order is invalid for {operation}/{library}")
        grouped.setdefault(operation, set()).add(library)
    if len(grouped) != 128 or any(libraries != LIBRARIES for libraries in grouped.values()):
        raise ValueError("each of 128 operations must have both library rows")
    return len(rows)


def validate(path: Path) -> int:
    return validate_text(path.read_text(encoding="utf-8"))


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: validate_corpus_legal_benchmark.py OUTPUT", file=sys.stderr)
        return 2
    try:
        count = validate(Path(sys.argv[1]))
    except (OSError, ValueError):
        return 1
    print(f"corpus legal benchmark OK: rows={count};operations=128")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
