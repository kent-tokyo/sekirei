#!/usr/bin/env python3
"""Freeze Q21r's post-Q21p candidate-vs-material development match."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

from validate_nnue_output_metadata import validate as validate_nnue_output


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    q21p = read(args.q21p_decision)
    preflight = read(args.preflight)
    openings_manifest = read(args.openings_manifest)
    require(
        q21p.get("schema") == "sekirei.q21p-depth7-ranking-pilot-decision.v4"
        and q21p.get("status") == "pass"
        and q21p.get("q21p_success") is True
        and q21p.get("development_match_authorized") is True
        and q21p.get("q20_authorized") is False,
        "Q21r requires Q21p v4 PASS with development-match-only authorization",
    )
    require(
        q21p.get("artifacts", {}).get("candidate", {}).get("sha256") == sha256(args.candidate),
        "candidate differs from Q21p PASS artifact",
    )
    require(
        preflight.get("schema") == "sekirei.gate-resource-preflight.v1"
        and preflight.get("mode") == "formal_preflight"
        and preflight.get("formal_measurement_eligible") is True
        and preflight.get("verdict") == "pass",
        "formal resource preflight did not pass",
    )
    openings = [line for line in args.openings.read_text(encoding="utf-8").splitlines() if line]
    require(len(openings) == 16 and len(set(openings)) == 16, "Q21r requires 16 unique openings")
    require(
        openings_manifest.get("schema") == "sekirei.gate-opening-corpus.v1"
        and openings_manifest.get("positions") == 16
        and openings_manifest.get("openings_sha256") == sha256(args.openings),
        "opening manifest mismatch",
    )
    recorded_exclusions = {
        (row.get("path"), row.get("sha256"))
        for row in openings_manifest.get("excluded_source_manifests", [])
    }
    required_exclusions = {(str(path), sha256(path)) for path in args.required_exclusion_manifest}
    require(required_exclusions <= recorded_exclusions, "opening manifest omits a required prior-source exclusion")
    verified = validate_nnue_output(args.candidate, "residual-material")
    return {
        "schema": "sekirei.q21r-development-match-plan.v1",
        "status": "frozen_before_match",
        "diagnostic_only": True,
        "strength_claim": False,
        "screen_evidence": {"q21p_decision": bind(args.q21p_decision)},
        "runtime": {
            "preflight": bind(args.preflight),
            "engine": bind(args.engine),
            "runner": bind(args.runner),
            "candidate": bind(args.candidate),
            "candidate_metadata": bind(Path(str(verified["metadata"]))),
            "candidate_mode": verified["mode"],
            "baseline": "material",
        },
        "openings": {
            **bind(args.openings),
            "manifest": bind(args.openings_manifest),
            "positions": 16,
            "required_exclusions": [bind(path) for path in args.required_exclusion_manifest],
        },
        "protocol": {
            "batch": 1,
            "games_per_position": 2,
            "games": 32,
            "byoyomi_ms": 1000,
            "max_moves": 512,
            "parallel": 1,
            "engine_options": {
                "Threads": "1",
                "SpecTopN": "0",
                "MultiPV": "1",
                "UseBook": "false",
                "SearchMode": "Speculative",
                "Hash": "64",
            },
            "candidate_options": {
                "EvalFile": str(args.candidate),
                "NnueOutput": "residual-material",
            },
            "reject_below": 0.40,
            "pass_at_or_above": 0.60,
            "middle_band": "exactly one additional fresh 16-opening batch is allowed",
        },
        "authorization": {
            "batch_1_below_0_40": "reject candidate and complete Q21r",
            "batch_1_at_or_above_0_60": "complete Q21r and authorize Q20",
            "batch_1_middle_band": "authorize exactly one fresh 32-game batch",
            "q20_before_match": False,
        },
        "tools": {"preparer": bind(Path(__file__).resolve()), "finalizer": bind(args.finalizer)},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in (
        "q21p-decision", "preflight", "engine", "runner", "candidate", "openings",
        "openings-manifest", "finalizer", "output",
    ):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--required-exclusion-manifest", type=Path, action="append", default=[])
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"status": document["status"], "games": 32}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
