#!/usr/bin/env python3
"""Finalize a terminal local strength gate without promoting candidate adoption."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


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


def terminal_verdict(state: dict) -> str:
    running = [shard for shard in state.get("shards", []) if shard.get("status") == "running"]
    if running:
        raise ValueError("gate is still running")
    verdict = state.get("decisive_verdict")
    if verdict in {"PASS", "FAIL"}:
        return verdict
    pending = [shard for shard in state.get("shards", []) if shard.get("status") == "pending"]
    failed = [shard for shard in state.get("shards", []) if shard.get("status") == "failed"]
    if pending or failed:
        raise ValueError("gate has not reached a terminal verdict")
    return "INCONCLUSIVE"


def build(execution_manifest_path: Path, state_path: Path) -> dict:
    execution = json.loads(execution_manifest_path.read_text(encoding="utf-8"))
    state = json.loads(state_path.read_text(encoding="utf-8"))
    if execution.get("schema") != "sekirei.strength-gate-execution.v1":
        raise ValueError("unsupported execution manifest")
    protocol = execution.get("protocol")
    if not isinstance(protocol, dict) or protocol.get("games_per_position") != 2 \
            or protocol.get("positions") != 200 or protocol.get("max_games") != 400:
        raise ValueError("execution manifest does not describe the frozen formal gate")
    sprt = protocol.get("sprt")
    if not isinstance(sprt, dict) or sprt.get("variant") != "trinomial" \
            or sprt.get("paired_by_id") is not True:
        raise ValueError("execution manifest does not use paired trinomial SPRT")
    evaluation = execution.get("evaluation")
    if not isinstance(evaluation, dict):
        raise ValueError("execution manifest lacks NNUE evaluation binding")
    for arm in ("candidate", "baseline"):
        binding = evaluation.get(arm)
        if not isinstance(binding, dict) or binding.get("mode") not in {"absolute", "residual-material"}:
            raise ValueError(f"execution manifest lacks valid {arm} NNUE output mode")
    verdict = terminal_verdict(state)
    run_directory = Path(execution["run_directory"])
    combined = run_directory / "combined.json"
    combined_jsonl = run_directory / "combined.jsonl"
    summary = json.loads(combined.read_text(encoding="utf-8"))
    records = [json.loads(line) for line in combined_jsonl.read_text(encoding="utf-8").splitlines() if line.strip()]
    expected_records = int(state.get("confirmed_prefix", 0)) * 2
    if len(records) != expected_records:
        raise ValueError("combined record count does not match completed colour-reversed pairs")
    if len(records) > protocol["max_games"]:
        raise ValueError("combined record count exceeds frozen max-games")
    if any(record.get("result") not in {"candidate_win", "baseline_win", "draw"} for record in records):
        raise ValueError("combined records contain an invalid result")
    pair_counts: dict[str, int] = {}
    for record in records:
        pair_id = record.get("id")
        if not isinstance(pair_id, str) or not pair_id:
            raise ValueError("combined records are missing pair ids")
        pair_counts[pair_id] = pair_counts.get(pair_id, 0) + 1
    if any(count != 2 for count in pair_counts.values()):
        raise ValueError("combined records contain incomplete or duplicate colour pairs")
    return {
        "schema": "sekirei.strength-gate-final.v1",
        "diagnostic_only": False,
        "execution_manifest": artifact(execution_manifest_path),
        "state": artifact(state_path),
        "combined": artifact(combined),
        "combined_jsonl": artifact(combined_jsonl),
        "verdict": verdict,
        "completed_colour_reversed_pairs": state.get("confirmed_prefix"),
        "games": len(records),
        "summary": summary,
        "candidate": execution["candidate"],
        "baseline": execution["baseline"],
        "evaluation": evaluation,
        "protocol": protocol,
        "claims": {
            "strength": "permitted_only_as_this_gate_verdict",
            "candidate_adoption": "not_automatic",
            "competitor_superiority": "not_established",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execution-manifest", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = build(args.execution_manifest, args.state)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {document['verdict']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
