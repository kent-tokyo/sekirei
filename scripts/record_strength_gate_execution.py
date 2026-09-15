#!/usr/bin/env python3
"""Bind a running local strength gate to immutable executable and plan evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
from pathlib import Path

from validate_nnue_output_metadata import validate as validate_nnue_output


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(path: Path) -> dict:
    if not path.is_file():
        raise ValueError(f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha256(path), "bytes": path.stat().st_size}


def rustc_version() -> str:
    result = subprocess.run(["rustc", "-Vv"], capture_output=True, text=True, check=False)
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def git_identity() -> dict[str, object]:
    revision = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=False)
    dirty = subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True, check=False)
    return {
        "revision": revision.stdout.strip() if revision.returncode == 0 else "unknown",
        "dirty": bool(dirty.stdout.strip()) if dirty.returncode == 0 else None,
    }


def build(plan_path: Path, binary: Path, preflight: Path, state: Path) -> dict:
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    state_document = json.loads(state.read_text(encoding="utf-8"))
    if plan.get("schema") != "sekirei.strength-gate-plan.v1":
        raise ValueError("unsupported strength gate plan")
    if plan.get("status") != "planned" or plan.get("strength_claim") is not False:
        raise ValueError("plan is not a non-claiming planned gate")
    protocol = plan.get("protocol")
    if not isinstance(protocol, dict) or protocol.get("games_per_position") != 2:
        raise ValueError("plan does not describe colour-reversed pairs")
    if protocol.get("positions") != 200 or protocol.get("max_games") != 400:
        raise ValueError("plan does not describe the frozen 200-position/400-game gate")
    sprt = protocol.get("sprt")
    if not isinstance(sprt, dict) or (
        sprt.get("variant") != "trinomial" or sprt.get("paired_by_id") is not True
    ):
        raise ValueError("plan does not describe paired trinomial SPRT")
    shards = state_document.get("shards")
    if not isinstance(shards, list) or not shards:
        raise ValueError("gate state has no shards")
    cfg = state_document.get("cfg")
    if not isinstance(cfg, dict):
        raise ValueError("gate state lacks launch configuration")
    if cfg.get("threads") != 1 or cfg.get("parallel") != 1 or cfg.get("byoyomi") != protocol.get("byoyomi_ms"):
        raise ValueError("gate state does not match frozen thread/parallel/time contract")
    if cfg.get("corpus") != plan["openings"].get("path"):
        raise ValueError("gate state corpus does not match frozen openings")
    evaluation = plan.get("evaluation")
    if not isinstance(evaluation, dict):
        raise ValueError("plan lacks NNUE evaluation binding")
    option1, option2 = cfg.get("option1"), cfg.get("option2")
    if not isinstance(option1, list) or not isinstance(option2, list):
        raise ValueError("gate state lacks per-arm options")
    if f"EvalFile={plan['candidate'].get('path')}" not in option1:
        raise ValueError("candidate EvalFile is not bound to engine1")
    if f"EvalFile={plan['baseline'].get('path')}" not in option2:
        raise ValueError("baseline EvalFile is not bound to engine2")
    for arm, options in (("candidate", option1), ("baseline", option2)):
        binding = evaluation.get(arm)
        if not isinstance(binding, dict):
            raise ValueError(f"plan lacks {arm} evaluation binding")
        mode = binding.get("mode")
        weight = binding.get("weights")
        if not isinstance(mode, str) or not isinstance(weight, dict):
            raise ValueError(f"invalid {arm} evaluation binding")
        weight_path = weight.get("path")
        if not isinstance(weight_path, str) or not weight_path:
            raise ValueError(f"invalid {arm} evaluation weight path")
        if weight_path != plan[arm].get("path"):
            raise ValueError(f"{arm} evaluation weight does not match plan")
        try:
            verified = validate_nnue_output(Path(weight_path), mode)
        except (OSError, ValueError) as error:
            raise ValueError(f"{arm} NNUE metadata is invalid: {error}") from error
        if binding.get("checkpoint_hash") != verified.get("hash"):
            raise ValueError(f"{arm} NNUE checkpoint hash does not match plan")
        if f"NnueOutput={mode}" not in options:
            raise ValueError(f"{arm} NnueOutput is not bound to its engine arm")
    if any(f"{key}={value}" not in option1 or f"{key}={value}" not in option2
           for key, value in protocol["engine_options"].items()):
        raise ValueError("gate state options do not match frozen protocol")
    return {
        "schema": "sekirei.strength-gate-execution.v1",
        "status": "running",
        "strength_claim": False,
        "plan": artifact(plan_path),
        "binary": artifact(binary),
        "preflight": artifact(preflight),
        "candidate": plan["candidate"],
        "baseline": plan["baseline"],
        "evaluation": evaluation,
        "protocol": protocol,
        "run_directory": str(state.parent),
        "state_at_recording": {
            "artifact": artifact(state),
            "confirmed_prefix": state_document.get("confirmed_prefix"),
            "shards": len(shards),
            "decisive_verdict": state_document.get("decisive_verdict"),
        },
        "environment": {"platform": platform.platform(), "rustc_vv": rustc_version()},
        "source": git_identity(),
        "launch": {"cfg": cfg},
        "claims": {
            "strength": "not_permitted_until_terminal_gate_verdict",
            "candidate_adoption": "not_established",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--preflight", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = build(args.plan, args.binary, args.preflight, args.state)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {document['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
