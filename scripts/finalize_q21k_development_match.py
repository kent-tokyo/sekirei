#!/usr/bin/env python3
"""Validate and finalize the frozen Q21k development match."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def rows(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def finalize(plan_path: Path, result_path: Path, records_path: Path, csa_path: Path) -> dict:
    plan, result, records, csa = read(plan_path), read(result_path), rows(records_path), read(csa_path)
    require(plan.get("schema") == "sekirei.q21k-development-match-plan.v1", "unexpected plan schema")
    for section, key in (
        ("runtime", "preflight"), ("runtime", "engine"), ("runtime", "runner"),
        ("runtime", "candidate"), ("runtime", "candidate_metadata"),
        ("screen_evidence", "cost"), ("screen_evidence", "content_preregistration"),
        ("screen_evidence", "content_screen"), ("openings", None), ("openings", "manifest"),
    ):
        item = plan[section] if key is None else plan[section][key]
        require(sha256(Path(item["path"])) == item["sha256"], f"frozen artifact changed: {section}/{key}")
    require(result.get("status") == "complete" and result.get("games") == 32, "match is incomplete")
    wins, draws, losses = (int(result.get(key, -1)) for key in ("engine1_wins", "draws", "engine2_wins"))
    require(min(wins, draws, losses) >= 0 and wins + draws + losses == 32, "invalid W/D/L")
    require(not result.get("invalid_games") and not result.get("artifact_write_failures"), "invalid match artifacts")
    expected = plan["protocol"]["engine_options"]
    for name, value in expected.items():
        require(result.get("engine1_options", {}).get(name) == value, f"candidate {name} mismatch")
        require(result.get("engine2_options", {}).get(name) == value, f"baseline {name} mismatch")
    candidate_options = plan["protocol"]["candidate_options"]
    for name, value in candidate_options.items():
        require(result.get("engine1_options", {}).get(name) == value, f"candidate {name} mismatch")
    require("EvalFile" not in result.get("engine2_options", {}), "baseline unexpectedly loaded weights")
    require(
        result.get("engine1_eval_file_acknowledgement")
        == f"info string NNUE weights loaded from {plan['runtime']['candidate']['path']}",
        "candidate weight-load acknowledgement mismatch",
    )
    require(result.get("engine2_eval_file_acknowledgement") is None, "baseline acknowledged weights")
    require(len(records) == 32, "record count mismatch")
    counts = Counter(row.get("id") for row in records)
    require(len(counts) == 16 and set(counts.values()) == {2}, "opening pairs are incomplete")
    outcomes = Counter(row.get("result") for row in records)
    require(
        outcomes
        == Counter({"candidate_win": wins, "draw": draws, "baseline_win": losses}),
        "per-game outcomes do not match aggregate W/D/L",
    )
    require(csa.get("status") == "complete" and csa.get("games_completed") == 32, "CSA manifest incomplete")
    written = csa.get("csa_games_written", [])
    skipped = csa.get("csa_games_skipped", [])
    require(len(written) + len(skipped) == 32, "CSA artifact accounting is incomplete")
    require(
        all("MaxMoves: max-moves draw" in item for item in skipped),
        "CSA artifacts contain an unexplained skip",
    )
    require(len(skipped) <= draws, "more max-moves CSA skips than recorded draws")
    score = (wins + 0.5 * draws) / 32
    if score < plan["protocol"]["reject_below"]:
        verdict, q20, extra = "REJECTED_DEVELOPMENT_MATCH", False, False
    elif score >= plan["protocol"]["pass_at_or_above"]:
        verdict, q20, extra = "PASS_TO_Q20", True, False
    else:
        verdict, q20, extra = "HOLD_ONE_ADDITIONAL_BATCH", False, True
    return {
        "schema": "sekirei.q21k-development-match-decision.v1",
        "status": "complete",
        "strength_claim": False,
        "result": {"wins": wins, "draws": draws, "losses": losses, "score": score, "verdict": verdict},
        "csa": {
            "written": len(written),
            "skipped_max_moves_draws": len(skipped),
            "note": "max-moves draws remain valid match results but are not valid CSA training games",
        },
        "decision": {
            "q21k_completed": not extra,
            "q20_authorized": q20,
            "additional_batch_required": extra,
            "candidate_formally_adopted": False,
        },
        "artifacts": {
            "plan": {"path": str(plan_path), "sha256": sha256(plan_path)},
            "result": {"path": str(result_path), "sha256": sha256(result_path)},
            "records": {"path": str(records_path), "sha256": sha256(records_path)},
            "csa_manifest": {"path": str(csa_path), "sha256": sha256(csa_path)},
        },
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
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(document["result"] | document["decision"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
