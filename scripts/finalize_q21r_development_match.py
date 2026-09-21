#!/usr/bin/env python3
"""Validate Q21r's frozen 32-game development match and decide its next gate."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def rows(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def verify_bound(path_record: dict[str, Any], label: str) -> None:
    path = Path(path_record["path"])
    require(path.is_file() and sha256(path) == path_record["sha256"], f"frozen artifact changed: {label}")


def finalize(plan_path: Path, result_path: Path, records_path: Path, csa_path: Path) -> dict[str, Any]:
    plan, result, records, csa = read(plan_path), read(result_path), rows(records_path), read(csa_path)
    require(plan.get("schema") == "sekirei.q21r-development-match-plan.v1", "unexpected plan schema")
    for section, key in (
        ("runtime", "preflight"), ("runtime", "engine"), ("runtime", "runner"),
        ("runtime", "candidate"), ("runtime", "candidate_metadata"),
        ("screen_evidence", "q21p_decision"), ("openings", None), ("openings", "manifest"),
    ):
        item = plan[section] if key is None else plan[section][key]
        verify_bound(item, f"{section}/{key}")
    for index, item in enumerate(plan["openings"]["required_exclusions"]):
        verify_bound(item, f"required_exclusion/{index}")
    expected_games = plan["protocol"]["games"]
    require(result.get("status") == "complete" and result.get("games") == expected_games, "match is incomplete")
    wins, draws, losses = (int(result.get(key, -1)) for key in ("engine1_wins", "draws", "engine2_wins"))
    require(min(wins, draws, losses) >= 0 and wins + draws + losses == expected_games, "invalid W/D/L")
    require(not result.get("invalid_games") and not result.get("artifact_write_failures"), "invalid match artifacts")
    expected = plan["protocol"]["engine_options"]
    for name, value in expected.items():
        require(result.get("engine1_options", {}).get(name) == value, f"candidate {name} mismatch")
        require(result.get("engine2_options", {}).get(name) == value, f"baseline {name} mismatch")
    for name, value in plan["protocol"]["candidate_options"].items():
        require(result.get("engine1_options", {}).get(name) == value, f"candidate {name} mismatch")
    require("EvalFile" not in result.get("engine2_options", {}), "baseline unexpectedly loaded weights")
    require(
        result.get("engine1_eval_file_acknowledgement")
        == f"info string NNUE weights loaded from {plan['runtime']['candidate']['path']}",
        "candidate weight-load acknowledgement mismatch",
    )
    require(result.get("engine2_eval_file_acknowledgement") is None, "baseline acknowledged weights")
    require(len(records) == expected_games, "record count mismatch")
    counts = Counter(row.get("id") for row in records)
    require(
        len(counts) == plan["openings"]["positions"]
        and set(counts.values()) == {plan["protocol"]["games_per_position"]},
        "opening pairs are incomplete",
    )
    outcomes = Counter(row.get("result") for row in records)
    require(
        outcomes == Counter({"candidate_win": wins, "draw": draws, "baseline_win": losses}),
        "per-game outcomes do not match aggregate W/D/L",
    )
    require(
        csa.get("status") == "complete" and csa.get("games_completed") == expected_games,
        "CSA manifest incomplete",
    )
    written = csa.get("csa_games_written", [])
    skipped = csa.get("csa_games_skipped", [])
    require(len(written) + len(skipped) == expected_games, "CSA artifact accounting is incomplete")
    require(all("MaxMoves: max-moves draw" in item for item in skipped), "CSA contains an unexplained skip")
    require(len(skipped) <= draws, "more max-moves CSA skips than recorded draws")
    score = (wins + 0.5 * draws) / expected_games
    if score < plan["protocol"]["reject_below"]:
        verdict, q20, extra, complete = "REJECTED_DEVELOPMENT_MATCH", False, False, True
    elif score >= plan["protocol"]["pass_at_or_above"]:
        verdict, q20, extra, complete = "PASS_TO_Q20", True, False, True
    else:
        verdict, q20, extra, complete = "HOLD_ONE_ADDITIONAL_BATCH", False, True, False
    return {
        "schema": "sekirei.q21r-development-match-decision.v1",
        "status": "complete" if complete else "batch_1_complete_more_required",
        "strength_claim": False,
        "result": {
            "wins": wins,
            "draws": draws,
            "losses": losses,
            "score": score,
            "verdict": verdict,
        },
        "decision": {
            "q21r_completed": complete,
            "q20_authorized": q20,
            "additional_batch_required": extra,
            "candidate_formally_adopted": False,
        },
        "csa": {
            "written": len(written),
            "skipped_max_moves_draws": len(skipped),
            "note": "max-moves draws remain valid results but are not valid CSA training games",
        },
        "artifacts": {
            "plan": {"path": str(plan_path), "sha256": sha256(plan_path)},
            "result": {"path": str(result_path), "sha256": sha256(result_path)},
            "records": {"path": str(records_path), "sha256": sha256(records_path)},
            "csa_manifest": {"path": str(csa_path), "sha256": sha256(csa_path)},
            "finalizer": {"path": str(Path(__file__).resolve()), "sha256": sha256(Path(__file__).resolve())},
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
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(document["result"] | document["decision"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
