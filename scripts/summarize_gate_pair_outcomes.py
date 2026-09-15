#!/usr/bin/env python3
"""Reconcile local-gate pair results with their two saved kifus.

The output is diagnostic-only.  It identifies which color-reversed opening
pairs were candidate sweeps, splits, or losses; it does not estimate strength
or turn a played move into a training label.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import Counter
from pathlib import Path


RESULT = re.compile(r"^# Result: (Engine1 Win|Engine2 Win|Draw)")
ENGINE1 = re.compile(r"^# Engine1: .* \((Black|White)\)$")
NNUE_ACK = re.compile(r"^# Engine([12]) NNUE: info string NNUE output mode: (absolute|residual-material)$")
SHARD = re.compile(r"^shard_(\d{4})\.json$")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def kifu_result(path: Path) -> str:
    for line in path.read_text(encoding="utf-8").splitlines():
        match = RESULT.match(line)
        if match:
            return match.group(1)
    raise ValueError(f"{path}: missing Result header")


def kifu_contract(path: Path) -> dict[str, str]:
    """Read the immutable per-game facts needed for a colour-pair claim."""
    engine1_color = None
    result = None
    position = None
    nnue_acknowledgements: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = ENGINE1.match(line)
        if match:
            engine1_color = match.group(1).lower()
        match = RESULT.match(line)
        if match:
            result = match.group(1)
        if line.startswith("position "):
            position = line
        match = NNUE_ACK.match(line)
        if match:
            nnue_acknowledgements[match.group(1)] = match.group(2)
    if engine1_color is None or result is None or position is None:
        raise ValueError(f"{path}: missing Engine1 color, Result, or position")
    # The initial position is everything before the optional replay history.
    initial = position.split(" moves ", 1)[0]
    return {
        "candidate_color": engine1_color,
        "result": result,
        "initial_position": initial,
        "nnue_acknowledgements": nnue_acknowledgements,
    }


def candidate_outcome(result: str) -> str:
    return {
        "Engine1 Win": "candidate_win",
        "Engine2 Win": "candidate_loss",
        "Draw": "draw",
    }[result]


def pair_class(outcomes: list[str]) -> str:
    wins = outcomes.count("candidate_win")
    losses = outcomes.count("candidate_loss")
    if wins == 2:
        return "candidate_sweep_win"
    if losses == 2:
        return "candidate_sweep_loss"
    if wins == 1 and losses == 1:
        return "split"
    return "draw_involved"


def summarize(run_dir: Path, execution_manifest: Path | None = None) -> dict:
    rows = []
    errors = []
    for path in sorted(run_dir.glob("shard_*.json")):
        match = SHARD.match(path.name)
        if not match:
            continue
        shard_id = match.group(1)
        try:
            shard = json.loads(path.read_text(encoding="utf-8"))
            kifu_dir = run_dir / f"shard_{shard_id}_kifu"
            kifus = sorted(kifu_dir.glob("game*.txt"))
            if len(kifus) != 2 or shard.get("games") != 2:
                raise ValueError("expected exactly two game records")
            contracts = [kifu_contract(kifu) for kifu in kifus]
            outcomes = [candidate_outcome(contract["result"]) for contract in contracts]
            if {contract["candidate_color"] for contract in contracts} != {"black", "white"}:
                raise ValueError("pair does not reverse candidate color")
            if len({contract["initial_position"] for contract in contracts}) != 1:
                raise ValueError("pair does not share the same initial position")
            expected = {
                "candidate_win": shard.get("engine1_wins"),
                "candidate_loss": shard.get("engine2_wins"),
                "draw": shard.get("draws"),
            }
            observed = Counter(outcomes)
            if any(observed[name] != expected[name] for name in expected):
                raise ValueError(f"kifu/shard mismatch: observed={dict(observed)} expected={expected}")
            rows.append({
                "pair_id": f"shard_{shard_id}",
                "outcomes": outcomes,
                "pair_class": pair_class(outcomes),
                "candidate_colors": [contract["candidate_color"] for contract in contracts],
                "initial_position": contracts[0]["initial_position"],
                "initial_position_sha256": hashlib.sha256(
                    contracts[0]["initial_position"].encode("utf-8")
                ).hexdigest(),
                "kifu": [str(kifu) for kifu in kifus],
                "shard": {"path": str(path), "sha256": sha256(path)},
            })
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            errors.append({"shard": path.name, "error": str(exc)})
    classes = Counter(row["pair_class"] for row in rows)
    execution = None
    if execution_manifest is not None:
        execution = json.loads(execution_manifest.read_text(encoding="utf-8"))
        launch = execution.get("launch", {}) if isinstance(execution, dict) else {}
        cfg = launch.get("cfg") if isinstance(launch, dict) else None
        evaluation = execution.get("evaluation") if isinstance(execution, dict) else None
        if not isinstance(cfg, dict) or not isinstance(evaluation, dict):
            errors.append({"execution": str(execution_manifest), "error": "missing launch/evaluation binding"})
        else:
            for arm, option_key in (("candidate", "option1"), ("baseline", "option2")):
                binding = evaluation.get(arm)
                options = cfg.get(option_key)
                if not isinstance(binding, dict) or not isinstance(options, list) or f"NnueOutput={binding.get('mode')}" not in options:
                    errors.append({"execution": str(execution_manifest), "error": f"{arm} NnueOutput mismatch"})
                    continue
                expected = binding.get("mode")
                engine_number = "1" if arm == "candidate" else "2"
                if any(contract["nnue_acknowledgements"].get(engine_number) != expected
                       for row in rows for contract in (kifu_contract(Path(path)) for path in row["kifu"])):
                    errors.append({"execution": str(execution_manifest), "error": f"{arm} NNUE acknowledgement mismatch"})
    return {
        "schema": "sekirei.gate-pair-outcome-summary.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "candidate_engine": 1,
        "execution_manifest": (
            {"path": str(execution_manifest), "sha256": sha256(execution_manifest)}
            if execution_manifest is not None else None
        ),
        "pairs": rows,
        "summary": {"pairs": len(rows), "classes": dict(sorted(classes.items())), "errors": len(errors)},
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--execution-manifest", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = summarize(args.run_dir, args.execution_manifest)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {document['summary']}")
    return 0 if not document["errors"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
