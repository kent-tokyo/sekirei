#!/usr/bin/env python3
"""Merge Q21n's single preregistered timeout retry into the full deep artifact."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def merge(args: argparse.Namespace) -> dict[str, Any]:
    amendment = json.loads(args.amendment.read_text(encoding="utf-8"))
    initial = json.loads(args.initial_deep.read_text(encoding="utf-8"))
    retry = json.loads(args.retry_deep.read_text(encoding="utf-8"))
    require(
        amendment.get("schema") == "sekirei.q21n-timeout-retry-amendment.v1"
        and amendment.get("status") == "frozen_before_retry_label",
        "unexpected timeout amendment",
    )
    require(amendment["inputs"]["initial_deep"]["sha256"] == sha256(args.initial_deep), "initial SHA mismatch")
    require(
        retry.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and len(retry.get("rows", [])) == 1,
        "retry artifact must contain exactly one root-ranking row",
    )
    target = amendment["retry"]["position_id"]
    replacement = retry["rows"][0]
    require(replacement.get("id") == target, "retry measured a different parent")
    require(
        replacement.get("candidate_prefix_complete") is True
        and replacement.get("complete_legal_root_set") is True,
        "retry parent is still incomplete",
    )
    for field in ("binary_sha256", "weights_sha256", "nnue_output"):
        require(
            retry.get("teacher", {}).get(field) == initial.get("teacher", {}).get(field),
            f"retry {field} differs from initial artifact",
        )
    require(
        retry.get("contract", {}).get("depth") == amendment["retry"]["depth"]
        and retry.get("contract", {}).get("root_candidate_limit") == amendment["retry"]["root_candidate_limit"],
        "retry search contract differs from amendment",
    )
    rows = []
    replaced = 0
    for row in initial.get("rows", []):
        if row.get("id") == target:
            rows.append(replacement)
            replaced += 1
        else:
            require(
                row.get("candidate_prefix_complete") is True
                and row.get("complete_legal_root_set") is True,
                f"unexpected second incomplete parent: {row.get('id')}",
            )
            rows.append(row)
    require(replaced == 1 and len(rows) == 18, "retry replacement count mismatch")
    result = dict(initial)
    result["contract"] = {
        **initial["contract"],
        "root_candidate_mode": "complete_legal_set",
        "complete_legal_root_set": True,
    }
    result["rows"] = rows
    result["q21n_timeout_retry"] = {
        "amendment": bind(args.amendment),
        "initial_deep": bind(args.initial_deep),
        "retry_deep": bind(args.retry_deep),
        "position_id": target,
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--amendment", type=Path, required=True)
    parser.add_argument("--initial-deep", type=Path, required=True)
    parser.add_argument("--retry-deep", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = merge(args)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(f"wrote {args.output}: 18/18 completed root prefixes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
