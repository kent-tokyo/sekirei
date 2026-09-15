#!/usr/bin/env python3
"""Freeze a diagnostic candidate without implying evaluator adoption or Elo."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(path: Path) -> dict:
    if not path.is_file():
        raise ValueError(f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha256(path), "bytes": path.stat().st_size}


def validation_artifact(path: Path | None) -> tuple[str, dict | None]:
    if path is None:
        return "not_run", None
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.floodgate-holdout-regression.v1":
        raise ValueError("independent validation has an unsupported schema")
    if document.get("status") != "clean_for_this_diagnostic":
        raise ValueError("independent validation is not clean for this diagnostic")
    return "clean_for_this_diagnostic", artifact(path)


def build(candidate: Path, baseline: Path, binary: Path, corpus: Path, preflight: Path,
          independent_validation: Path | None = None) -> dict:
    preflight_data = json.loads(preflight.read_text(encoding="utf-8"))
    validation_status, validation = validation_artifact(independent_validation)
    requirements = {
        "independent_validation": validation_status,
        "paired_strength_gate": "not_run",
        "formal_adoption": False,
    }
    if validation is not None:
        requirements["independent_validation_artifact"] = validation
    return {
        "schema": "sekirei.fg4-candidate-manifest.v1",
        "status": "diagnostic_candidate_not_adopted",
        "diagnostic_only": True,
        "candidate": artifact(candidate),
        "baseline": artifact(baseline),
        "binary": artifact(binary),
        "corpus": artifact(corpus),
        "execution": {
            "threads": 1,
            "rayon_num_threads": 1,
            "spec_top_n": 0,
            "nodes_completed": 20_000,
            "next_nodes": [100_000, 1_000_000],
        },
        "preflight": {
            "artifact": artifact(preflight),
            "verdict": preflight_data.get("verdict", "unknown"),
        },
        "adoption_requirements": requirements,
        "claims": {
            "strength": "not_permitted",
            "candidate_adoption": "not_established",
            "competitor_superiority": "not_established",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--preflight", type=Path, required=True)
    parser.add_argument("--independent-validation", type=Path,
                        help="clean diagnostic-only hold-out regression artifact")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = build(args.candidate, args.baseline, args.binary, args.corpus, args.preflight,
                     args.independent_validation)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {document['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
