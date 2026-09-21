#!/usr/bin/env python3
"""Freeze Q21k's post-screen 32-game development-match contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from validate_nnue_output_metadata import validate as validate_nnue_output


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def bind(path: Path) -> dict:
    return {"path": str(path), "sha256": sha256(path)}


def prepare(args: argparse.Namespace) -> dict:
    cost = read(args.cost_decision)
    content_prereg = read(args.content_preregistration)
    content_screen = read(args.content_screen)
    preflight = read(args.preflight)
    openings_manifest = read(args.openings_manifest)
    if cost.get("schema") != "sekirei.q21k-eval-cache-decision.v1" or cost.get("status") != "screen_pass":
        raise ValueError("Q21k cost track has not passed")
    if content_prereg.get("schema") != "sekirei.q21k-phase-reweight-preregistration.v1":
        raise ValueError("unexpected Q21k content preregistration")
    if content_screen.get("schema") != "sekirei.q21i-imitation-screen.v1" or content_screen.get("status") != "screen_pass":
        raise ValueError("Q21k content track has not passed")
    if (
        preflight.get("schema") != "sekirei.gate-resource-preflight.v1"
        or preflight.get("mode") != "formal_preflight"
        or preflight.get("formal_measurement_eligible") is not True
        or preflight.get("verdict") != "pass"
    ):
        raise ValueError("formal resource preflight did not pass")
    openings = [line for line in args.openings.read_text(encoding="utf-8").splitlines() if line]
    if len(openings) != 16 or len(set(openings)) != 16:
        raise ValueError("Q21k development match requires 16 unique openings")
    if openings_manifest.get("positions") != 16 or openings_manifest.get("openings_sha256") != sha256(args.openings):
        raise ValueError("opening manifest mismatch")
    verified = validate_nnue_output(args.candidate, "residual-material")
    return {
        "schema": "sekirei.q21k-development-match-plan.v1",
        "status": "frozen_before_match",
        "diagnostic_only": True,
        "strength_claim": False,
        "screen_evidence": {
            "cost": bind(args.cost_decision),
            "content_preregistration": bind(args.content_preregistration),
            "content_screen": bind(args.content_screen),
        },
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
        },
        "protocol": {
            "games_per_position": 2,
            "games": 32,
            "byoyomi_ms": 1000,
            "max_moves": 512,
            "engine_options": {
                "Threads": "1",
                "SpecTopN": "0",
                "MultiPV": "1",
                "UseBook": "false",
                "SearchMode": "Speculative",
                "Hash": "64",
            },
            "candidate_options": {"EvalFile": str(args.candidate), "NnueOutput": "residual-material"},
            "reject_below": 0.40,
            "pass_at_or_above": 0.60,
            "middle_band": "exactly one additional fresh 16-opening batch is allowed",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cost-decision", "content-preregistration", "content-screen", "preflight", "engine", "runner", "candidate", "openings", "openings-manifest", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": document["status"], "games": 32}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
