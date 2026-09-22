#!/usr/bin/env python3
"""Freeze Q27's Q30-reduced-versus-material development match."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

from validate_nnue_output_metadata import validate as validate_nnue_output


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    screen, preflight, openings_manifest = (
        read(args.q30_screen_decision), read(args.preflight), read(args.openings_manifest)
    )
    require(
        screen.get("schema") == "sekirei.q30-efficiency-validation-decision.v1"
        and screen.get("status") == "pass"
        and screen.get("q27_authorized") is True
        and screen.get("q20_authorized") is False,
        "Q27 requires the Q30 sealed-holdout PASS only",
    )
    require(
        screen["artifacts"]["reduced_candidate"]["sha256"] == sha256(args.candidate),
        "candidate differs from the Q30 PASS artifact",
    )
    require(
        preflight.get("schema") == "sekirei.gate-resource-preflight.v1"
        and preflight.get("mode") == "formal_preflight"
        and preflight.get("formal_measurement_eligible") is True
        and preflight.get("verdict") == "pass",
        "formal resource preflight did not pass",
    )
    openings = [line for line in args.openings.read_text(encoding="utf-8").splitlines() if line.strip()]
    require(len(openings) == 16 and len(openings) == len(set(openings)), "Q27 needs 16 unique openings")
    require(
        openings_manifest.get("schema") == "sekirei.gate-opening-corpus.v1"
        and openings_manifest.get("positions") == 16
        and openings_manifest.get("openings_sha256") == sha256(args.openings),
        "Q27 opening manifest mismatch",
    )
    exclusions = {(row.get("path"), row.get("sha256")) for row in openings_manifest.get("excluded_source_manifests", [])}
    expected_exclusions = {(str(path), sha256(path)) for path in args.required_exclusion_manifest}
    require(expected_exclusions <= exclusions, "opening manifest omits required source exclusions")
    verified = validate_nnue_output(args.candidate, "residual-material")
    return {
        "schema": "sekirei.q27-q30-reduced-match-plan.v1",
        "status": "frozen_before_match",
        "diagnostic_only": True,
        "strength_claim": False,
        "screen_evidence": {"q30_screen_decision": bind(args.q30_screen_decision)},
        "runtime": {
            "preflight": bind(args.preflight), "runner": bind(args.runner),
            "candidate_engine": bind(args.candidate_engine), "material_engine": bind(args.material_engine),
            "candidate": bind(args.candidate), "candidate_metadata": bind(Path(str(verified["metadata"]))),
            "candidate_mode": verified["mode"], "baseline": "material",
        },
        "openings": {
            **bind(args.openings), "manifest": bind(args.openings_manifest), "positions": 16,
            "required_exclusions": [bind(path) for path in args.required_exclusion_manifest],
        },
        "protocol": {
            "games_per_position": 2, "games": 32, "byoyomi_ms": 1000, "max_moves": 512,
            "parallel": 1, "threads": 1, "spec_top_n": 0,
            "engine_options": {"Threads": "1", "SpecTopN": "0", "MultiPV": "1", "UseBook": "false", "SearchMode": "Speculative", "Hash": "64"},
            "candidate_options": {"EvalFile": str(args.candidate), "NnueOutput": "residual-material"},
            "pass_at_or_above": 0.60, "reject_below": 0.40,
            "middle_band": "inconclusive; use a new run ID and fresh openings for any later retry",
        },
        "authorization": {"q20_before_match": False, "q20_after_pass": True, "candidate_adoption": False},
        "tools": {"preparer": bind(Path(__file__).resolve()), "finalizer": bind(args.finalizer)},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in (
        "q30-screen-decision", "preflight", "candidate-engine", "material-engine", "runner",
        "candidate", "openings", "openings-manifest", "finalizer", "output",
    ):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--required-exclusion-manifest", type=Path, action="append", default=[])
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": document["status"], "games": document["protocol"]["games"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
