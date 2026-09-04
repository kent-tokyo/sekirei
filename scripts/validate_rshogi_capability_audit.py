#!/usr/bin/env python3
"""Validate the pinned, documentation-only rshogi capability audit."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_AUDIT = ROOT / "scripts" / "rshogi_capability_audit.toml"
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
CAPABILITIES = {
    "nnue_variants",
    "lazy_smp",
    "lock_free_tt",
    "adaptive_time_management",
    "usi",
    "sprt_and_tournament_tools",
}
STATUSES = {"documented", "not_observed", "measurement_required"}


def validate_audit(path: Path) -> int:
    with path.open("rb") as stream:
        document = tomllib.load(stream)
    source = document.get("source")
    if not isinstance(source, dict):
        raise ValueError("source table is required")
    if not str(source.get("repository", "")).startswith("https://github.com/"):
        raise ValueError("source.repository must be an HTTPS GitHub URL")
    if source.get("ref") != "main":
        raise ValueError("source.ref must identify the audited branch")
    if not SHA_RE.fullmatch(str(source.get("commit", ""))):
        raise ValueError("source.commit must be a 40-character SHA")
    if source.get("evidence") != "README.md":
        raise ValueError("source.evidence must be README.md")

    rows = document.get("capability")
    if not isinstance(rows, list) or not rows:
        raise ValueError("capability must be a non-empty array")
    names = set()
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("every capability row must be a table")
        name = row.get("name")
        if name not in CAPABILITIES or name in names:
            raise ValueError(f"invalid or duplicate capability: {name!r}")
        names.add(name)
        if row.get("status") not in STATUSES:
            raise ValueError(f"{name}: invalid status")
        if not row.get("evidence") or not row.get("note"):
            raise ValueError(f"{name}: evidence and note are required")
    if names != CAPABILITIES:
        raise ValueError(f"capability set mismatch: {sorted(names ^ CAPABILITIES)}")

    sekirei = document.get("sekirei")
    if not isinstance(sekirei, dict) or set(sekirei) != CAPABILITIES:
        raise ValueError("sekirei must classify every audited capability")
    return len(rows)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("audit", nargs="?", type=Path, default=DEFAULT_AUDIT)
    args = parser.parse_args()
    try:
        count = validate_audit(args.audit)
    except (OSError, tomllib.TOMLDecodeError, ValueError) as error:
        print(f"rshogi capability audit INVALID: {error}", file=sys.stderr)
        return 1
    print(f"rshogi capability audit OK: capabilities={count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

