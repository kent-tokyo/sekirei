#!/usr/bin/env python3
"""Validate Q27's frozen Q30-reduced-versus-material match artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def rows(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def verify(binding: dict[str, str], label: str) -> None:
    path = Path(binding["path"])
    require(path.is_file() and sha256(path) == binding["sha256"], f"frozen artifact changed: {label}")


def finalize(plan_path: Path, result_path: Path, records_path: Path, csa_path: Path) -> dict[str, Any]:
    plan, result, records, csa = read(plan_path), read(result_path), rows(records_path), read(csa_path)
    require(plan.get("schema") == "sekirei.q27-q30-reduced-match-plan.v1", "unexpected Q27 plan")
    for section, key in (("screen_evidence", "q30_screen_decision"), ("runtime", "preflight"),
                         ("runtime", "runner"), ("runtime", "candidate_engine"),
                         ("runtime", "material_engine"), ("runtime", "candidate"),
                         ("runtime", "candidate_metadata"), ("openings", None), ("openings", "manifest")):
        verify(plan[section] if key is None else plan[section][key], f"{section}/{key}")
    for index, binding in enumerate(plan["openings"]["required_exclusions"]):
        verify(binding, f"required_exclusion/{index}")
    protocol = plan["protocol"]
    expected_games = protocol["games"]
    require(result.get("status") == "complete" and result.get("games") == expected_games, "Q27 match incomplete")
    wins, draws, losses = (int(result.get(key, -1)) for key in ("engine1_wins", "draws", "engine2_wins"))
    require(min(wins, draws, losses) >= 0 and wins + draws + losses == expected_games, "invalid Q27 W/D/L")
    require(not result.get("invalid_games") and not result.get("artifact_write_failures"), "Q27 has invalid game artifacts")
    for name, value in protocol["engine_options"].items():
        require(result.get("engine1_options", {}).get(name) == value, f"candidate {name} mismatch")
        require(result.get("engine2_options", {}).get(name) == value, f"material {name} mismatch")
    for name, value in protocol["candidate_options"].items():
        require(result.get("engine1_options", {}).get(name) == value, f"candidate {name} mismatch")
    require("EvalFile" not in result.get("engine2_options", {}), "material arm loaded weights")
    acknowledgement = f"info string NNUE weights loaded from {plan['runtime']['candidate']['path']}"
    require(result.get("engine1_eval_file_acknowledgement") == acknowledgement, "candidate load acknowledgement mismatch")
    require(result.get("engine2_eval_file_acknowledgement") is None, "material acknowledged weights")
    require(len(records) == expected_games, "Q27 record count mismatch")
    counts = Counter(row.get("id") for row in records)
    require(len(counts) == plan["openings"]["positions"] and set(counts.values()) == {2}, "incomplete opening pairs")
    outcomes = Counter(row.get("result") for row in records)
    require(outcomes == Counter({"candidate_win": wins, "draw": draws, "baseline_win": losses}), "Q27 outcomes mismatch")
    require(csa.get("status") == "complete" and csa.get("games_completed") == expected_games, "CSA manifest incomplete")
    score = (wins + 0.5 * draws) / expected_games
    verdict = "PASS_TO_Q20" if score >= protocol["pass_at_or_above"] else ("REJECTED" if score < protocol["reject_below"] else "INCONCLUSIVE")
    return {
        "schema": "sekirei.q27-q30-reduced-match-decision.v1", "status": "complete",
        "strength_claim": False,
        "result": {"wins": wins, "draws": draws, "losses": losses, "score": score, "verdict": verdict},
        "decision": {"q27_completed": True, "q20_authorized": verdict == "PASS_TO_Q20", "candidate_formally_adopted": False},
        "artifacts": {name: {"path": str(path), "sha256": sha256(path)} for name, path in {
            "plan": plan_path, "result": result_path, "records": records_path, "csa_manifest": csa_path,
            "finalizer": Path(__file__).resolve(),
        }.items()},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--result", type=Path, required=True)
    parser.add_argument("--records", type=Path, required=True)
    parser.add_argument("--csa-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = finalize(args.plan, args.result, args.records, args.csa_manifest)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(document["result"] | document["decision"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
