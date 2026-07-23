#!/usr/bin/env python3
"""Verify every sha256 in docs/weights_registry.toml against the actual file
on disk. A hand-typed or truncated hash in a provenance manifest is worse
than a missing one -- it reads as "verified" while silently not being.

Usage: python3 scripts/verify_weights_registry.py
Exits non-zero if any weight_path's `sha256` field doesn't match the file,
or (informationally) if a file referenced by weight_path no longer exists.
"""
import hashlib
import sys
import tomllib

REGISTRY = "docs/weights_registry.toml"


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    with open(REGISTRY, "rb") as f:
        registry = tomllib.load(f)

    mismatches = []
    missing = []
    checked = 0
    for w in registry["weight"]:
        path = w.get("weight_path", "")
        reg_hash = w.get("sha256")
        # Skip entries whose weight_path names a checkpoint family
        # (e.g. "...epoch{1,2,3}.bin") rather than one real file.
        if not reg_hash or "{" in path:
            continue
        try:
            actual = sha256_of(path)
        except FileNotFoundError:
            missing.append(path)
            continue
        checked += 1
        if len(reg_hash) != 64:
            mismatches.append((path, reg_hash, actual, "wrong length"))
        elif reg_hash != actual:
            mismatches.append((path, reg_hash, actual, "mismatch"))

    if missing:
        print(f"NOTE: {len(missing)} weight_path(s) no longer exist on disk:")
        for p in missing:
            print(f"  {p}")

    if mismatches:
        print(f"FAIL: {len(mismatches)}/{checked} sha256 field(s) don't match the file:")
        for path, reg, actual, why in mismatches:
            print(f"  {path} ({why})\n    registry: {reg}\n    actual:   {actual}")
        sys.exit(1)

    print(f"OK: {checked} sha256 field(s) verified against disk.")


if __name__ == "__main__":
    main()
