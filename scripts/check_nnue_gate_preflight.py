#!/usr/bin/env python3
"""Fail closed before registering a new own-NNUE strength gate.

The caller supplies a reviewed ledger and the latest received exclusion closure.
This cannot prove that undisclosed historical positions do not exist.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def require(value: bool, message: str) -> None:
    if not value:
        raise ValueError(message)


def preflight(plan: dict, ledger: dict, closure: dict) -> dict:
    require(ledger.get("schema") == "sekirei.nnue-candidate-ledger.v1", "ledger schema")
    require(closure.get("all_manifest_hashes_verified") is True,
            "unverified exclusion closure")
    entries = ledger.get("entries")
    require(isinstance(entries, list) and entries, "empty candidate ledger")
    pins = plan.get("pins", {})
    candidate = pins.get(plan.get("candidate"), {})
    baseline = pins.get(plan.get("baseline"), {})
    require(candidate.get("sha256") and baseline.get("sha256"), "weight pins missing")
    require(candidate["sha256"] != baseline["sha256"], "identical candidate and baseline")
    for entry in entries:
        require(entry.get("candidate_sha256") and entry.get("status")
                and entry.get("evidence"), "incomplete ledger row")
        if entry["candidate_sha256"] == candidate["sha256"]:
            raise ValueError(f"candidate already recorded: {entry['status']}")
    manifest_pin = pins.get("root_manifest", {})
    require(manifest_pin.get("path") and manifest_pin.get("sha256"),
            "root manifest pin missing")
    manifest_path = Path(manifest_pin["path"])
    require(sha(manifest_path) == manifest_pin["sha256"], "root manifest changed")
    manifest = json.loads(manifest_path.read_text())
    require(manifest.get("status") == "complete", "root manifest incomplete")
    scheme = manifest.get("identity_scheme")
    roots = [row.get("identity_sha256") for row in manifest.get("positions", [])]
    require(roots and all(isinstance(value, str) and len(value) == 64 for value in roots)
            and len(set(roots)) == len(roots), "root identities missing or duplicate")
    closure_rows = closure.get("manifests")
    require(isinstance(closure_rows, list) and closure_rows, "empty closure")
    expected = {row["sha256"] for row in closure_rows}
    received = {row["sha256"] for row in manifest.get("exclusions", [])}
    require(expected <= received, "formal roots omit known exclusion manifests")
    require(len(expected) == len(closure_rows), "duplicate closure manifest")
    overlap = set()
    for row in closure_rows:
        path = Path(row["path"])
        require(sha(path) == row["sha256"], "exclusion manifest changed")
        document = json.loads(path.read_text())
        require(document.get("identity_scheme") == scheme
                and document.get("complete_for_scope") is True,
                "exclusion manifest contract changed")
        overlap.update(set(roots) & set(document["identities"]))
    require(not overlap, "formal roots overlap historical exclusions")
    return {"status": "PASS", "candidate_sha256": candidate["sha256"],
            "baseline_sha256": baseline["sha256"], "roots": len(roots),
            "exclusion_manifests": len(closure_rows),
            "scope": "received ledger and exclusion closure only"}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--closure", type=Path, required=True)
    args = parser.parse_args()
    ledger = json.loads(args.ledger.read_text())
    received = ledger.get("latest_known_closure", {})
    require(received.get("path") == str(args.closure)
            and received.get("sha256") == sha(args.closure),
            "latest received closure differs from ledger")
    result = preflight(json.loads(args.plan.read_text()), ledger,
                       json.loads(args.closure.read_text()))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
