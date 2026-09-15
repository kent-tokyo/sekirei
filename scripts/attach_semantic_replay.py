#!/usr/bin/env python3
"""Attach verified semantic-replay results to a review-manifest copy."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


def attach(manifest: dict, replay: dict) -> dict:
    if manifest.get("schema") != "sekirei.floodgate-review-manifest.v1":
        raise ValueError("unexpected review manifest schema")
    rows = replay.get("results")
    if not isinstance(rows, list):
        raise ValueError("replay results must be a list")
    by_id = {row.get("id"): row for row in rows if isinstance(row, dict)}
    if len(by_id) != len(rows):
        raise ValueError("replay results contain duplicate or missing ids")

    result = json.loads(json.dumps(manifest))
    pairs = result.get("pairs")
    if not isinstance(pairs, list) or len(pairs) != len(rows):
        raise ValueError("replay result count does not match manifest pairs")
    for pair in pairs:
        pair_id = pair.get("id")
        row = by_id.pop(pair_id, None)
        if row is None or row.get("status") != "verified":
            raise ValueError(f"semantic replay is not verified for {pair_id}")
        if row.get("csa_moves") != pair.get("csa", {}).get("moves"):
            raise ValueError(f"CSA move count mismatch for {pair_id}")
        if row.get("search_records") != pair.get("analysis", {}).get("search_records"):
            raise ValueError(f"search record count mismatch for {pair_id}")
        pair["evidence"]["semantic_replay"] = "verified"
        pair["evidence_status"]["semantic_replay"] = "verified"
        pair["semantic_replay"] = {
            "status": "verified",
            "csa_moves": row["csa_moves"],
            "search_records": row["search_records"],
        }
    if by_id:
        raise ValueError("replay results contain an unknown manifest id")
    result["evidence_summary"]["semantic_replay_verified"] = len(pairs)
    result["evidence_summary"]["semantic_replay_unknown"] = 0
    result["semantic_replay"] = {
        "status": "verified",
        "results_sha256": replay.get("results_sha256", "unknown"),
    }
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("replay", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        replay = json.loads(args.replay.read_text(encoding="utf-8"))
        result = attach(manifest, replay)
        args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        parser.error(str(exc))
    print(f"wrote semantic-replay manifest: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
