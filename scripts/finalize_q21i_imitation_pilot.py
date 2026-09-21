#!/usr/bin/env python3
"""Validate and finalize the preregistered Q21i imitation pilot.

This tool deliberately keeps the ranking screen and the development match as
separate pieces of evidence.  Passing the former does not authorize Q20 when
the latter rejects the candidate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


SCHEMA = "sekirei.q21i-imitation-decision.v1"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def read_jsonl(path: Path) -> list[dict]:
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not all(isinstance(row, dict) for row in rows):
        raise ValueError(f"{path} must contain JSON objects")
    return rows


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def finalize(
    preregistration_path: Path,
    ranking_screen_path: Path,
    preflight_path: Path,
    match_result_path: Path,
    match_records_path: Path,
    csa_manifest_path: Path,
    openings_path: Path,
    openings_manifest_path: Path,
    candidate_path: Path,
    engine_path: Path,
) -> dict:
    preregistration = read_json(preregistration_path)
    ranking = read_json(ranking_screen_path)
    preflight = read_json(preflight_path)
    match = read_json(match_result_path)
    records = read_jsonl(match_records_path)
    csa = read_json(csa_manifest_path)
    openings_manifest = read_json(openings_manifest_path)

    require(preregistration.get("schema") == "sekirei.q21i-imitation-preregistration.v1",
            "unexpected preregistration schema")
    require(ranking.get("schema") == "sekirei.q21i-imitation-screen.v1",
            "unexpected ranking-screen schema")
    require(ranking.get("status") == "screen_pass",
            "development match is not authorized without a ranking pass")
    require(ranking.get("checks", {}).get("rank_loss_reduction_pass") is True,
            "rank-loss reduction did not pass")
    require(ranking.get("checks", {}).get("major_blunders_nonincrease") is True,
            "major-blunder count increased")
    require(preflight.get("schema") == "sekirei.gate-resource-preflight.v1",
            "unexpected resource-preflight schema")
    require(preflight.get("mode") == "formal_preflight",
            "development match used a relaxed resource preflight")
    require(preflight.get("formal_measurement_eligible") is True and preflight.get("verdict") == "pass",
            "resource preflight did not authorize a formal measurement")
    require(preregistration.get("binaries", {}).get("engine", {}).get("sha256") == sha256(engine_path),
            "engine hash differs from preregistration")

    contract = preregistration["development_match_screen"]
    expected_positions = int(contract["new_start_positions"])
    expected_games = int(contract["maximum_games"])
    openings = [line.strip() for line in openings_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    require(len(openings) == expected_positions, "opening count differs from preregistration")
    require(len(set(openings)) == len(openings), "development openings are not unique")
    require(openings_manifest.get("positions") == expected_positions,
            "opening manifest position count mismatch")
    require(openings_manifest.get("openings_sha256") == sha256(openings_path),
            "opening file hash mismatch")

    require(match.get("status") == "complete", "development match is incomplete")
    require(match.get("games") == expected_games, "development match game count mismatch")
    wins = int(match.get("engine1_wins", -1))
    draws = int(match.get("draws", -1))
    losses = int(match.get("engine2_wins", -1))
    require(min(wins, draws, losses) >= 0 and wins + draws + losses == expected_games,
            "development match W/D/L totals are invalid")
    require(not match.get("invalid_games"), "development match contains invalid games")
    require(not match.get("artifact_write_failures"), "development match has artifact failures")

    candidate_options = match.get("engine1_options", {})
    baseline_options = match.get("engine2_options", {})
    require(candidate_options.get("EvalFile") == str(candidate_path), "candidate EvalFile mismatch")
    require(candidate_options.get("NnueOutput") == "residual-material", "candidate NNUE mode mismatch")
    for name, expected in (("Threads", "1"), ("SpecTopN", "0"), ("MultiPV", "1"),
                           ("UseBook", "false"), ("SearchMode", "Speculative")):
        require(candidate_options.get(name) == expected, f"candidate {name} mismatch")
        require(baseline_options.get(name) == expected, f"baseline {name} mismatch")
    require("EvalFile" not in baseline_options and "NnueOutput" not in baseline_options,
            "baseline is not the material-only arm")
    expected_ack = f"info string NNUE weights loaded from {candidate_path}"
    require(match.get("engine1_eval_file_acknowledgement") == expected_ack,
            "candidate load acknowledgement mismatch")
    require(match.get("engine2_eval_file_acknowledgement") is None,
            "material arm unexpectedly acknowledged NNUE weights")

    require(len(records) == expected_games, "per-game record count mismatch")
    pair_counts = Counter(row.get("id") for row in records)
    require(len(pair_counts) == expected_positions and set(pair_counts.values()) == {2},
            "records do not contain exactly two color-reversed games per opening")
    require(csa.get("status") == "complete", "CSA manifest is incomplete")
    require(csa.get("games_completed") == expected_games, "CSA completed-game count mismatch")
    require(len(csa.get("csa_games_written", [])) == expected_games,
            "not every game has a CSA artifact")
    require(not csa.get("csa_games_skipped"), "CSA manifest skipped games")

    score = (wins + 0.5 * draws) / expected_games
    pass_score = float(contract["pass_score"])
    reject_below = float(contract["reject_below"])
    if score >= pass_score:
        verdict = "PASS_TO_Q20"
        q20_authorized = True
        second_set_allowed = False
    elif score < reject_below:
        verdict = "REJECTED_DEVELOPMENT_MATCH"
        q20_authorized = False
        second_set_allowed = False
    else:
        verdict = "HOLD_SECOND_SET_ALLOWED"
        q20_authorized = False
        second_set_allowed = True

    return {
        "schema": SCHEMA,
        "status": "complete",
        "strength_claim": False,
        "single_factor": preregistration.get("single_factor"),
        "ranking_screen": {
            "status": ranking["status"],
            "parents": ranking.get("parents"),
            "baseline_mean_parent_rank_loss_cp": ranking["baseline"]["mean_parent_rank_loss_cp"],
            "candidate_mean_parent_rank_loss_cp": ranking["candidate"]["mean_parent_rank_loss_cp"],
            "rank_loss_reduction": ranking["rank_loss_reduction"],
            "baseline_major_blunders_ge_300cp": ranking["baseline"]["major_blunders_ge_300cp"],
            "candidate_major_blunders_ge_300cp": ranking["candidate"]["major_blunders_ge_300cp"],
        },
        "development_match": {
            "verdict": verdict,
            "wins": wins,
            "draws": draws,
            "losses": losses,
            "score": score,
            "pass_score": pass_score,
            "reject_below": reject_below,
            "second_set_allowed": second_set_allowed,
            "elo_diff_descriptive_only": match.get("elo_diff"),
            "elo_ci_low_descriptive_only": match.get("elo_ci_low"),
            "elo_ci_high_descriptive_only": match.get("elo_ci_high"),
            "unique_games": match.get("unique_games"),
            "invalid_games": len(match.get("invalid_games", [])),
            "csa_games": len(csa["csa_games_written"]),
        },
        "decision": {
            "candidate_adopted": False,
            "q20_authorized": q20_authorized,
            "recipe_b_allowed": False,
            "recipe_b_reason": "contingency was allowed only if recipe A failed the ranking screen",
            "interpretation": (
                "The candidate improved the frozen teacher-ranking diagnostic but failed the "
                "independent same-time development match. Ranking improvement is not playing-strength evidence."
            ),
        },
        "artifacts": {
            "preregistration": {"path": str(preregistration_path), "sha256": sha256(preregistration_path)},
            "ranking_screen": {"path": str(ranking_screen_path), "sha256": sha256(ranking_screen_path)},
            "preflight": {"path": str(preflight_path), "sha256": sha256(preflight_path)},
            "engine": {"path": str(engine_path), "sha256": sha256(engine_path)},
            "candidate": {"path": str(candidate_path), "sha256": sha256(candidate_path)},
            "openings": {"path": str(openings_path), "sha256": sha256(openings_path)},
            "openings_manifest": {"path": str(openings_manifest_path), "sha256": sha256(openings_manifest_path)},
            "match_result": {"path": str(match_result_path), "sha256": sha256(match_result_path)},
            "match_records": {"path": str(match_records_path), "sha256": sha256(match_records_path)},
            "csa_manifest": {"path": str(csa_manifest_path), "sha256": sha256(csa_manifest_path)},
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--ranking-screen", type=Path, required=True)
    parser.add_argument("--preflight", type=Path, required=True)
    parser.add_argument("--match-result", type=Path, required=True)
    parser.add_argument("--match-records", type=Path, required=True)
    parser.add_argument("--csa-manifest", type=Path, required=True)
    parser.add_argument("--openings", type=Path, required=True)
    parser.add_argument("--openings-manifest", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        decision = finalize(
            args.preregistration, args.ranking_screen, args.preflight, args.match_result, args.match_records,
            args.csa_manifest, args.openings, args.openings_manifest, args.candidate, args.engine,
        )
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(decision, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": decision["status"], "verdict": decision["development_match"]["verdict"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
