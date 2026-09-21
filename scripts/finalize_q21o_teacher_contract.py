#!/usr/bin/env python3
"""Choose and validate Q21o's stronger fixed-T teacher-search contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from pathlib import Path
from typing import Any


MATE_THRESHOLD_CP = 899_000


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def exact(result: dict[str, Any]) -> bool:
    return (
        result.get("completion") == "search_completed"
        and result.get("completed_iteration_valid") == "true"
        and result.get("completed_bound") == "exact"
        and result.get("pv_legal") is True
        and result.get("history_matches_expected") == "true"
        and isinstance(result.get("score_cp"), int)
    )


def ordinary(result: dict[str, Any]) -> bool:
    return exact(result) and abs(result["score_cp"]) < MATE_THRESHOLD_CP


def signature(result: dict[str, Any]) -> tuple[Any, ...]:
    return result.get("bestmove"), result.get("score_cp"), result.get("depth"), result.get("completed_bound")


def regret_class(regret: int) -> str:
    if regret >= 300:
        return "major"
    if regret >= 100:
        return "moderate"
    return "small"


def calibration_arm(repeats: list[dict[str, Any]], old_moves: list[str]) -> dict[str, Any]:
    valid = all(
        ordinary(repeat["free"])
        and set(repeat["fixed_depth3_top"]) == set(old_moves)
        and all(ordinary(result) for result in repeat["fixed_depth3_top"].values())
        for repeat in repeats
    )
    deterministic = valid and all(
        signature(repeat["free"]) == signature(repeats[0]["free"])
        and all(
            signature(repeat["fixed_depth3_top"][move])
            == signature(repeats[0]["fixed_depth3_top"][move])
            for move in old_moves
        )
        for repeat in repeats[1:]
    )
    regrets = []
    elapsed = []
    if valid:
        for repeat in repeats:
            regrets.append(
                repeat["free"]["score_cp"]
                - max(repeat["fixed_depth3_top"][move]["score_cp"] for move in old_moves)
            )
            elapsed.append(
                repeat["free"]["elapsed_ms"]
                + sum(repeat["fixed_depth3_top"][move]["elapsed_ms"] for move in old_moves)
            )
    return {
        "valid": valid,
        "deterministic": deterministic,
        "free_bestmove": repeats[0]["free"].get("bestmove"),
        "old_depth3_regret_cp": regrets[0] if deterministic else None,
        "old_depth3_regret_class": regret_class(regrets[0]) if deterministic else "incomplete",
        "elapsed_ms_per_repeat": elapsed,
    }


def ranking_scores(results: dict[str, dict[str, Any]]) -> dict[str, int]:
    require(results and all(ordinary(result) for result in results.values()), "ranking contains incomplete or mate-like result")
    return {move: result["score_cp"] for move, result in results.items()}


def top_set(scores: dict[str, int]) -> set[str]:
    best = max(scores.values())
    return {move for move, score in scores.items() if score == best}


def finalize(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    measurements = json.loads(args.measurements.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q21o-teacher-contract-preregistration.v1"
        and prereg.get("status") == "frozen_before_measurement",
        "unexpected Q21o preregistration",
    )
    require(
        measurements.get("schema") == "sekirei.q21o-teacher-contract-measurements.v1"
        and measurements.get("preregistration", {}).get("sha256") == sha256(args.preregistration),
        "measurements are not bound to the Q21o preregistration",
    )
    require(len(measurements.get("rows", [])) == 6, "Q21o requires six measured parents")
    rows = []
    arm_elapsed: dict[str, list[int]] = {name: [] for name in prereg["arms"]}
    all_deterministic = True
    for row in measurements["rows"]:
        selection = row["selection"]
        calibration = {
            arm: calibration_arm(repeats, selection["depth3_top_moves"])
            for arm, repeats in row["calibration"].items()
        }
        require(set(calibration) == set(prereg["arms"]), "calibration arm set mismatch")
        for arm, result in calibration.items():
            all_deterministic &= result["valid"] and result["deterministic"]
            arm_elapsed[arm].extend(result["elapsed_ms_per_repeat"])
        rankings = {arm: ranking_scores(results) for arm, results in row["ranking"].items()}
        require(set(rankings) == set(prereg["arms"]), "ranking arm set mismatch")
        require(all(set(scores) == set(row["candidate_moves"]) for scores in rankings.values()), "ranking candidate set mismatch")
        rows.append({"selection": selection, "calibration": calibration, "rankings": rankings})
    require(all_deterministic, "Q21o calibration was incomplete or non-deterministic")

    problem_rows = [row for row in rows if row["selection"]["role"] == "problem"]
    class_matches = all(
        row["calibration"]["depth7"]["old_depth3_regret_class"]
        == row["calibration"]["nodes3200k"]["old_depth3_regret_class"]
        for row in problem_rows
    )
    free_agreements = sum(
        row["calibration"]["depth7"]["free_bestmove"]
        == row["calibration"]["nodes3200k"]["free_bestmove"]
        for row in rows
    )
    fixed_nodes_eligible = class_matches and free_agreements >= 4
    arm_median_elapsed = {
        arm: statistics.median(values) for arm, values in arm_elapsed.items()
    }
    if fixed_nodes_eligible and arm_median_elapsed["nodes3200k"] < arm_median_elapsed["depth7"]:
        selected_arm = "nodes3200k"
    else:
        selected_arm = "depth7"
    peer_arm = "depth7" if selected_arm == "nodes3200k" else "nodes3200k"

    audited_rows = []
    for row in rows:
        selected_scores = row["rankings"][selected_arm]
        peer_scores = row["rankings"][peer_arm]
        depth7_scores = row["rankings"]["depth7"]
        selected_top = top_set(selected_scores)
        peer_regret = max(peer_scores.values()) - max(peer_scores[move] for move in selected_top)
        depth7_regret = max(depth7_scores.values()) - max(depth7_scores[move] for move in selected_top)
        audited_rows.append({
            **row["selection"],
            "selected_top_moves": sorted(selected_top),
            "peer_regret_cp": peer_regret,
            "depth7_regret_cp": depth7_regret,
            "old_depth3_depth7_regret_cp": row["calibration"]["depth7"]["old_depth3_regret_cp"],
        })
    old_major = sum(row["old_depth3_depth7_regret_cp"] >= 300 for row in audited_rows)
    selected_major_depth7 = sum(row["depth7_regret_cp"] >= 300 for row in audited_rows)
    selected_major_peer = sum(row["peer_regret_cp"] >= 300 for row in audited_rows)
    passed = (
        selected_major_depth7 == 0
        and selected_major_peer == 0
        and old_major > selected_major_depth7
    )
    return {
        "schema": "sekirei.q21o-teacher-contract-decision.v1",
        "status": "pass" if passed else "fail",
        "diagnostic_only": True,
        "strength_claim": False,
        "selected_contract": {
            "arm": selected_arm,
            **prereg["arms"][selected_arm],
            "threads": 1,
            "spec_top_n": 0,
            "nnue_output": "residual-material",
            "weights_sha256": prereg["measurement_contract"]["weights_sha256"],
            "binary_sha256": prereg["measurement_contract"]["binary_sha256"],
        },
        "selection_evidence": {
            "fixed_nodes_eligible": fixed_nodes_eligible,
            "problem_regret_classes_match": class_matches,
            "free_bestmove_agreements": free_agreements,
            "parents": len(rows),
            "median_calibration_elapsed_ms": arm_median_elapsed,
        },
        "label_reaudit": {
            "parents": len(rows),
            "old_depth3_major_depth7_regret_ge_300cp": old_major,
            "selected_label_major_depth7_regret_ge_300cp": selected_major_depth7,
            "selected_label_major_peer_regret_ge_300cp": selected_major_peer,
            "strict_major_regret_reduction": old_major > selected_major_depth7,
        },
        "development_match_authorized": False,
        "q20_authorized": False,
        "next_action": (
            "regenerate only a small ranking-label pilot under the selected teacher contract"
            if passed else
            "do not regenerate labels; the compared teacher contracts lack cross-contract robustness"
        ),
        "rows": audited_rows,
        "artifacts": {
            "preregistration": bind(args.preregistration),
            "measurements": bind(args.measurements),
            "finalizer": bind(Path(__file__).resolve()),
        },
    }


def report(document: dict[str, Any]) -> str:
    selected = document["selected_contract"]
    evidence = document["selection_evidence"]
    audit = document["label_reaudit"]
    return "\n".join((
        "# Q21o teacher-search contract",
        "",
        "This is fixed-T label QA, not a strength result.",
        "",
        f"- Status: `{document['status']}`.",
        f"- Selected: `{selected['arm']}` (depth={selected['max_depth']}, nodes={selected['nodes']}, Threads=1).",
        f"- Calibration median elapsed: depth7 {evidence['median_calibration_elapsed_ms']['depth7']:.1f}ms; "
        f"nodes3200k {evidence['median_calibration_elapsed_ms']['nodes3200k']:.1f}ms.",
        f"- Free bestmove agreement: {evidence['free_bestmove_agreements']}/{evidence['parents']}.",
        f"- Fixed-node eligible: {evidence['fixed_nodes_eligible']} "
        f"(problem regret classes match: {evidence['problem_regret_classes_match']}).",
        f"- Old depth-3 major depth-7 regret: {audit['old_depth3_major_depth7_regret_ge_300cp']}; "
        f"selected labels: {audit['selected_label_major_depth7_regret_ge_300cp']}.",
        f"- Selected-label major peer regret: {audit['selected_label_major_peer_regret_ge_300cp']}.",
        "- Development match and Q20 remain unauthorized.",
        "",
    ))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--measurements", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    try:
        document = finalize(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.report is not None:
        args.report.write_text(report(document), encoding="utf-8")
    print(json.dumps({
        "status": document["status"],
        "selected_contract": document["selected_contract"],
        "selection_evidence": document["selection_evidence"],
        "label_reaudit": document["label_reaudit"],
    }, sort_keys=True))
    return 0 if document["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
