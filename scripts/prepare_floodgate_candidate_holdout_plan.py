#!/usr/bin/env python3
"""Prepare a frozen, non-executing hold-out plan for the current candidate."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-candidate-holdout-plan.v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def prepare(candidate: dict, corpus: dict, split: dict, paths: dict[str, str]) -> dict:
    errors = []
    if candidate.get("schema") != "sekirei.floodgate-review-manifest.v1":
        errors.append("candidate.schema")
    contract = candidate.get("run_contract", {})
    version = contract.get("engine_version") if isinstance(contract, dict) else None
    if not isinstance(version, str) or not version:
        errors.append("candidate.engine_version")
    if corpus.get("schema") != "sekirei.floodgate-diagnostic-corpus.v1":
        errors.append("corpus.schema")
    if split.get("schema") != "sekirei.floodgate-diagnostic-corpus-split.v1":
        errors.append("split.schema")
    source_hash = split.get("source_corpus_sha256")
    if not isinstance(source_hash, str) or not source_hash:
        errors.append("split.source_corpus_sha256")
    if errors:
        raise ValueError(", ".join(errors))
    return {
        "schema": SCHEMA,
        "status": "planned",
        "execution_performed": False,
        "candidate": {
            "version": version,
            "manifest": paths["candidate"],
            "binary": candidate.get("provenance", {}).get("binary"),
            "source_revision": candidate.get("provenance", {}).get("source_revision"),
        },
        "corpus": {"path": paths["corpus"], "file_sha256": paths["corpus_sha256"], "source_corpus_sha256": source_hash},
        "split": {"path": paths["split"], "tuning_entries": len(split.get("tuning_entry_indices", [])),
                  "holdout_entries": len(split.get("holdout_entry_indices", []))},
        "protocol": {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative",
                     "preflight_required": True, "formal_gate": False},
        "claims": {"strength": "not_permitted", "candidate_adoption": "not_established"},
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate_manifest", type=Path)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("split", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = prepare(
        json.loads(args.candidate_manifest.read_text(encoding="utf-8")),
        json.loads(args.corpus.read_text(encoding="utf-8")),
        json.loads(args.split.read_text(encoding="utf-8")),
        {"candidate": str(args.candidate_manifest), "corpus": str(args.corpus), "split": str(args.split),
         "corpus_sha256": sha256(args.corpus)},
    )
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"candidate hold-out plan: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
