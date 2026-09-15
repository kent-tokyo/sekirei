#!/usr/bin/env python3
"""Keep only strict root-ranking pairs whose order survives a deeper teacher.

The filtered pair document intentionally keeps the established v1 schema so
the existing Rust trainer/auditor can validate it unchanged.  The depth-pair
provenance and hashes live in the companion manifest; no caller may silently
mistake this for a complete legal-root label set.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def digest(document: dict) -> str:
    return hashlib.sha256(
        json.dumps(document, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def pair_key(pair: dict) -> tuple[str, str, str]:
    values = (pair.get("parent_id"), pair.get("higher_move_usi"), pair.get("lower_move_usi"))
    if not all(isinstance(value, str) and value for value in values):
        raise ValueError("pair lacks parent_id/higher_move_usi/lower_move_usi")
    return values  # type: ignore[return-value]


def filter_pairs(shallow: dict, deep: dict) -> tuple[dict, dict]:
    for name, document in (("shallow", shallow), ("deep", deep)):
        if document.get("schema") != "sekirei.root-rank-pairs.v1" or document.get("diagnostic_only") is not True:
            raise ValueError(f"{name} input is not a diagnostic root-rank pair document")
    shallow_pairs = shallow.get("pairs")
    deep_pairs = deep.get("pairs")
    if not isinstance(shallow_pairs, list) or not isinstance(deep_pairs, list):
        raise ValueError("pair inputs need lists")
    stable = {pair_key(pair) for pair in deep_pairs if isinstance(pair, dict)}
    retained = [pair for pair in shallow_pairs if isinstance(pair, dict) and pair_key(pair) in stable]
    if not retained:
        raise ValueError("no pair order survived the deeper teacher")
    output = {key: value for key, value in shallow.items() if key != "pairs"}
    output["pairs"] = retained
    manifest = {
        "schema": "sekirei.stable-root-rank-pairs.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "rule": "retain shallow strict ordered pair only when identical directed pair is strict in deeper teacher",
        "shallow_input_sha256": digest(shallow),
        "deep_input_sha256": digest(deep),
        "shallow_pairs": len(shallow_pairs),
        "deep_pairs": len(deep_pairs),
        "retained_pairs": len(retained),
        "output_sha256": digest(output),
    }
    return output, manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shallow", type=Path, required=True)
    parser.add_argument("--deep", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest-output", type=Path, required=True)
    args = parser.parse_args()
    try:
        shallow = json.loads(args.shallow.read_text(encoding="utf-8"))
        deep = json.loads(args.deep.read_text(encoding="utf-8"))
        output, manifest = filter_pairs(shallow, deep)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    for path, document in ((args.output, output), (args.manifest_output, manifest)):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
