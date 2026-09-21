#!/usr/bin/env python3
"""Validate and summarize the preregistered Q21j failure audit.

The report keeps three questions separate:

* fixed-node material versus candidate-at-zero-residual validates that the
  latter isolates NNUE compute cost without changing the search result;
* fixed-time node ratios quantify that compute pressure;
* fixed-node candidate versus material comparisons describe evaluator-content
  differences and actual-move regret.

The selected positions are diagnostic evidence.  They cannot assign a causal
percentage of the original match losses and are not a strength gate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from collections import Counter, defaultdict
from pathlib import Path


ARMS = ("material", "candidate-cost-only", "candidate", "teacher")
CELLS = {
    "fixed_nodes_free": 3,
    "fixed_nodes_actual": 1,
    "fixed_time_free": 3,
}
MATE_LIKE_THRESHOLD = 899_000
MAJOR_CP = 300


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def read_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def median(values: list[int | float]) -> int | float:
    value = statistics.median(values)
    return int(value) if float(value).is_integer() else value


def ratio(numerator: int | float, denominator: int | float) -> float:
    if denominator <= 0:
        raise ValueError("ratio denominator must be positive")
    return float(numerator) / float(denominator)


def is_mate_like(score: int) -> bool:
    return abs(score) >= MATE_LIKE_THRESHOLD


def validate_input_hashes(preregistration: dict) -> None:
    for name, item in preregistration["inputs"].items():
        path = Path(item["path"])
        if not path.is_file():
            raise ValueError(f"missing preregistered {name}: {path}")
        actual = sha256(path)
        if actual != item["sha256"]:
            raise ValueError(f"preregistered {name} SHA mismatch: {actual}")


def measurement_key(row: dict) -> tuple[str, int, str, str]:
    return row["cell"], row["repetition"], row["position_id"], row["arm"]


def validate_measurements(corpus: dict, preregistration: dict, manifest: dict,
                          rows: list[dict], prereg_path: Path,
                          measurements_path: Path) -> dict:
    if corpus.get("schema") != "sekirei.q21j-failure-audit-corpus.v1":
        raise ValueError("unsupported Q21j corpus schema")
    if preregistration.get("schema") != "sekirei.q21j-failure-audit-preregistration.v1":
        raise ValueError("unsupported Q21j preregistration schema")
    if manifest.get("schema") != "sekirei.q21j-failure-audit-measurements.v1":
        raise ValueError("unsupported Q21j measurement schema")
    if manifest.get("status") != "complete":
        raise ValueError("Q21j measurement manifest is not complete")
    positions = corpus.get("positions", [])
    position_ids = [item["id"] for item in positions]
    if len(position_ids) != 32 or len(set(position_ids)) != 32:
        raise ValueError("Q21j requires exactly 32 unique positions")
    if preregistration.get("position_ids") != position_ids:
        raise ValueError("corpus positions differ from preregistration")
    if manifest.get("preregistration_sha256") != sha256(prereg_path):
        raise ValueError("preregistration SHA differs from manifest")
    if manifest.get("measurements_sha256") != sha256(measurements_path):
        raise ValueError("measurement SHA differs from manifest")

    expected = {
        (cell, repetition, position_id, arm)
        for cell, repetitions in CELLS.items()
        for repetition in range(repetitions)
        for position_id in position_ids
        for arm in ARMS
    }
    actual = [measurement_key(row) for row in rows]
    duplicates = [key for key, count in Counter(actual).items() if count > 1]
    missing = sorted(expected - set(actual))
    unexpected = sorted(set(actual) - expected)
    if duplicates or missing or unexpected or len(rows) != len(expected):
        raise ValueError(
            f"invalid measurement matrix: rows={len(rows)} expected={len(expected)} "
            f"duplicates={len(duplicates)} missing={len(missing)} unexpected={len(unexpected)}"
        )
    if manifest.get("measurements") != len(rows) or manifest.get("expected_measurements") != len(expected):
        raise ValueError("manifest measurement count differs from matrix")

    by_position = {item["id"]: item for item in positions}
    invalid = []
    for row in rows:
        result = row.get("result", {})
        checks = (
            result.get("completed_iteration_valid"),
            result.get("pv_legal"),
            result.get("pv_replay_preserves_input"),
            result.get("history_matches_expected"),
        )
        if not all(checks):
            invalid.append(measurement_key(row))
        source = by_position[row["position_id"]]
        expected_amount = (
            preregistration["contract"]["fixed_time_ms"]
            if row["cell"] == "fixed_time_free"
            else preregistration["contract"]["fixed_nodes"]
        )
        if row.get("amount") != expected_amount:
            raise ValueError(f"measurement amount differs from contract: {measurement_key(row)}")
        if (
            row.get("game_num") != source["game_num"]
            or row.get("game_result") != source["game_result"]
            or row.get("actual_move_usi") != source["position"]["actual_move_usi"]
        ):
            raise ValueError(f"measurement provenance differs from corpus: {measurement_key(row)}")
        if (
            row["cell"] == "fixed_nodes_actual"
            and result.get("bestmove") != source["position"]["actual_move_usi"]
        ):
            raise ValueError(f"forced-root result did not keep actual move: {measurement_key(row)}")
    if invalid:
        raise ValueError(f"{len(invalid)} measurement rows failed search/history validation")
    validate_input_hashes(preregistration)
    return {
        "valid": True,
        "positions": len(position_ids),
        "measurements": len(rows),
        "expected_measurements": len(expected),
        "duplicate_measurements": 0,
        "missing_measurements": 0,
        "unexpected_measurements": 0,
        "invalid_search_or_history_rows": 0,
        "cell_counts": dict(sorted(Counter(row["cell"] for row in rows).items())),
        "fixed_node_determinism_failures": 0,
    }


def dominant_bestmove(results: list[dict]) -> str:
    counts = Counter(item["bestmove"] for item in results)
    return sorted(counts, key=lambda move: (-counts[move], move))[0]


def aggregate_results(rows: list[dict]) -> dict[tuple[str, str, str], dict]:
    grouped: dict[tuple[str, str, str], list[dict]] = defaultdict(list)
    for row in rows:
        grouped[(row["cell"], row["position_id"], row["arm"])].append(row["result"])
    aggregates = {}
    for key, results in grouped.items():
        aggregates[key] = {
            "bestmove": dominant_bestmove(results),
            "bestmove_stable": len({item["bestmove"] for item in results}) == 1,
            "score_cp": median([item["score_cp"] for item in results]),
            "score_stable": len({item["score_cp"] for item in results}) == 1,
            "nodes": median([item["nodes"] for item in results]),
            "depth": median([item["depth"] for item in results]),
            "elapsed_ms": median([item["elapsed_ms"] for item in results]),
            "static_evaluations": median([item["static_evaluations"] for item in results]),
            "repetitions": len(results),
        }
    return aggregates


def pair_content(left: dict, right: dict) -> dict:
    mate_like = is_mate_like(int(left["score_cp"])) or is_mate_like(int(right["score_cp"]))
    difference = None if mate_like else abs(int(left["score_cp"]) - int(right["score_cp"]))
    return {
        "bestmove_disagrees": left["bestmove"] != right["bestmove"],
        "mate_like": mate_like,
        "absolute_score_difference_cp": difference,
        "large_score_difference": difference is not None and difference >= MAJOR_CP,
    }


def summarize_pair(position_rows: list[dict], pair_name: str) -> dict:
    normal = [item for item in position_rows if not item[pair_name]["mate_like"]]
    differences = [item[pair_name]["absolute_score_difference_cp"] for item in normal]
    return {
        "positions": len(position_rows),
        "bestmove_disagreements": sum(item[pair_name]["bestmove_disagrees"] for item in position_rows),
        "mate_like_positions_excluded_from_cp_metrics": len(position_rows) - len(normal),
        "normal_cp_positions": len(normal),
        "large_score_differences_ge_300cp": sum(item[pair_name]["large_score_difference"] for item in normal),
        "median_absolute_score_difference_cp": median(differences) if differences else None,
        "max_absolute_score_difference_cp": max(differences) if differences else None,
    }


def summarize_regret(items: list[dict], arm: str) -> dict:
    normal = [item for item in items if not item["regret"][arm]["mate_like"]]
    regrets = [item["regret"][arm]["cp"] for item in normal]
    return {
        "positions": len(items),
        "normal_cp_positions": len(normal),
        "mate_like_positions_excluded_from_cp_metrics": len(items) - len(normal),
        "major_regret_ge_300cp": sum(value >= MAJOR_CP for value in regrets),
        "zero_regret": sum(value == 0 for value in regrets),
        "median_regret_cp": median(regrets) if regrets else None,
        "max_regret_cp": max(regrets) if regrets else None,
    }


def summarize(corpus: dict, preregistration: dict, manifest: dict, rows: list[dict],
              prereg_path: Path, measurements_path: Path) -> dict:
    validation = validate_measurements(
        corpus, preregistration, manifest, rows, prereg_path, measurements_path
    )
    aggregates = aggregate_results(rows)
    fixed_node_failures = []
    for position_id in preregistration["position_ids"]:
        for arm in ARMS:
            item = aggregates[("fixed_nodes_free", position_id, arm)]
            if not item["bestmove_stable"] or not item["score_stable"]:
                fixed_node_failures.append({"position_id": position_id, "arm": arm})
    if fixed_node_failures:
        raise ValueError(f"fixed-node free search is not deterministic: {fixed_node_failures[:3]}")

    by_position = {item["id"]: item for item in corpus["positions"]}
    position_rows = []
    raw_identity = 0
    for row in rows:
        if row["cell"] != "fixed_nodes_free" or row["arm"] != "material":
            continue
        counterpart = next(
            item for item in rows
            if item["cell"] == row["cell"]
            and item["repetition"] == row["repetition"]
            and item["position_id"] == row["position_id"]
            and item["arm"] == "candidate-cost-only"
        )
        raw_identity += (
            row["result"]["bestmove"] == counterpart["result"]["bestmove"]
            and row["result"]["score_cp"] == counterpart["result"]["score_cp"]
        )

    for position_id in preregistration["position_ids"]:
        source = by_position[position_id]
        free = {arm: aggregates[("fixed_nodes_free", position_id, arm)] for arm in ARMS}
        actual = {arm: aggregates[("fixed_nodes_actual", position_id, arm)] for arm in ARMS}
        timed = {arm: aggregates[("fixed_time_free", position_id, arm)] for arm in ARMS}
        candidate_material = pair_content(free["candidate"], free["material"])
        teacher_material = pair_content(free["teacher"], free["material"])
        candidate_teacher = pair_content(free["candidate"], free["teacher"])
        node_ratio = ratio(timed["candidate-cost-only"]["nodes"], timed["material"]["nodes"])
        regret = {}
        for arm in ARMS:
            free_score = int(free[arm]["score_cp"])
            forced_score = int(actual[arm]["score_cp"])
            mate_like = is_mate_like(free_score) or is_mate_like(forced_score)
            regret[arm] = {
                "mate_like": mate_like,
                "cp": None if mate_like else max(0, free_score - forced_score),
                "free_score_cp": free_score,
                "forced_actual_score_cp": forced_score,
                "free_bestmove": free[arm]["bestmove"],
                "actual_move": source["position"]["actual_move_usi"],
            }
        content_signal = (
            candidate_material["bestmove_disagrees"]
            or candidate_material["large_score_difference"]
        )
        cost_signal = node_ratio <= 0.75
        classification = (
            "both" if content_signal and cost_signal
            else "content_only" if content_signal
            else "cost_only" if cost_signal
            else "neither"
        )
        position_rows.append({
            "position_id": position_id,
            "game_num": source["game_num"],
            "game_result": source["game_result"],
            "selection_rule": source["selection"]["rule"],
            "fixed_nodes": {
                arm: {name: value for name, value in free[arm].items() if name not in {"bestmove_stable", "score_stable"}}
                for arm in ARMS
            },
            "fixed_time": {
                arm: {name: value for name, value in timed[arm].items() if name not in {"bestmove_stable", "score_stable"}}
                for arm in ARMS
            },
            "candidate_material": candidate_material,
            "teacher_material": teacher_material,
            "candidate_teacher": candidate_teacher,
            "cost_only_material_node_ratio": node_ratio,
            "regret": regret,
            "signals": {"cost": cost_signal, "content": content_signal, "classification": classification},
        })

    identity_total = 32 * CELLS["fixed_nodes_free"]
    position_identity = sum(
        item["fixed_nodes"]["material"]["bestmove"]
        == item["fixed_nodes"]["candidate-cost-only"]["bestmove"]
        and item["fixed_nodes"]["material"]["score_cp"]
        == item["fixed_nodes"]["candidate-cost-only"]["score_cp"]
        for item in position_rows
    )
    identity_rate = raw_identity / identity_total
    node_ratios = [item["cost_only_material_node_ratio"] for item in position_rows]
    arm_node_medians = {
        arm: median([item["fixed_time"][arm]["nodes"] for item in position_rows]) for arm in ARMS
    }
    arm_depth_medians = {
        arm: median([item["fixed_time"][arm]["depth"] for item in position_rows]) for arm in ARMS
    }
    cost_isolated = identity_rate >= 0.95
    median_cost_ratio = float(median(node_ratios))
    cost_pressure = median_cost_ratio <= 0.75
    content_difference = any(item["signals"]["content"] for item in position_rows)
    classification_counts = dict(sorted(Counter(item["signals"]["classification"] for item in position_rows).items()))
    outcomes = {
        "candidate_losses": [item for item in position_rows if item["game_result"] == "baseline_win"],
        "candidate_wins": [item for item in position_rows if item["game_result"] == "candidate_win"],
    }

    candidate_loss_count = len(outcomes["candidate_losses"])
    if cost_pressure and content_difference:
        interpretation = (
            "Both evaluator-content differences and residual-NNUE compute-cost pressure are present "
            f"on the frozen decisions. The audit does not assign a causal percentage of the {candidate_loss_count} losses."
        )
    elif cost_pressure:
        interpretation = (
            "Residual-NNUE compute-cost pressure is present; the preregistered content signal is absent. "
            f"The audit does not assign a causal percentage of the {candidate_loss_count} losses."
        )
    elif content_difference:
        interpretation = (
            "Evaluator-content differences are present; the preregistered compute-cost threshold is not met. "
            f"The audit does not assign a causal percentage of the {candidate_loss_count} losses."
        )
    else:
        interpretation = (
            "Neither preregistered signal is established. The audit does not explain or assign the match losses."
        )

    summary = {
        "schema": "sekirei.q21j-failure-audit-summary.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_remains_rejected": True,
        "q20_authorized": False,
        "contract": preregistration["contract"],
        "validation": validation,
        "corpus": {
            "games": 32,
            "candidate_losses": len(outcomes["candidate_losses"]),
            "candidate_wins": len(outcomes["candidate_wins"]),
            "first_large_drop_positions": corpus["counts"]["first_large_drop"],
        },
        "cost_isolation": {
            "raw_fixed_node_score_and_bestmove_matches": raw_identity,
            "raw_fixed_node_comparisons": identity_total,
            "raw_identity_rate": identity_rate,
            "position_matches": position_identity,
            "positions": 32,
            "requirement_rate": 0.95,
            "pass": cost_isolated,
        },
        "fixed_time_cost": {
            "median_nodes_by_arm": arm_node_medians,
            "median_depth_by_arm": arm_depth_medians,
            "candidate_cost_only_material_node_ratio": {
                "median": median_cost_ratio,
                "min": min(node_ratios),
                "max": max(node_ratios),
                "threshold": 0.75,
                "positions_at_or_below_threshold": sum(value <= 0.75 for value in node_ratios),
                "pass": cost_pressure,
            },
        },
        "fixed_node_content": {
            "candidate_vs_material": summarize_pair(position_rows, "candidate_material"),
            "teacher_vs_material": summarize_pair(position_rows, "teacher_material"),
            "candidate_vs_teacher": summarize_pair(position_rows, "candidate_teacher"),
        },
        "actual_move_regret": {
            "all": {arm: summarize_regret(position_rows, arm) for arm in ARMS},
            **{
                outcome: {arm: summarize_regret(items, arm) for arm in ARMS}
                for outcome, items in outcomes.items()
            },
        },
        "signals": {
            "classification_counts": classification_counts,
            "by_outcome": {
                outcome: dict(sorted(Counter(item["signals"]["classification"] for item in items).items()))
                for outcome, items in outcomes.items()
            },
        },
        "conclusion": {
            "cost_isolated": cost_isolated,
            "cost_pressure": cost_pressure,
            "content_difference": content_difference,
            "interpretation": interpretation,
            "attribution_boundary": preregistration["contract"]["attribution_boundary"],
            "next_action": (
                "Do not rerun recipe B or start Q20. Improve evaluator content only under the same "
                "fixed-node contract and reduce NNUE cost under an output/state identity contract; a new "
                "candidate must pass a fresh development match before any independent gate."
            ),
        },
        "positions": position_rows,
    }
    return summary


def report_markdown(summary: dict) -> str:
    cost = summary["fixed_time_cost"]
    content = summary["fixed_node_content"]
    regret = summary["actual_move_regret"]
    conclusion = summary["conclusion"]
    nodes = cost["median_nodes_by_arm"]
    ratio_item = cost["candidate_cost_only_material_node_ratio"]
    candidate_content = content["candidate_vs_material"]
    lines = [
        "# NNUE failure audit",
        "",
        "This is a diagnostic attribution audit, not a strength gate.",
        "",
        "## Validation",
        "",
        f"- {summary['validation']['positions']} positions, {summary['validation']['measurements']} measurements, missing/duplicate/invalid: 0/0/0.",
        f"- Fixed-node material/cost-only identity: {summary['cost_isolation']['raw_fixed_node_score_and_bestmove_matches']}/{summary['cost_isolation']['raw_fixed_node_comparisons']} "
        f"({summary['cost_isolation']['raw_identity_rate']:.1%}); requirement >=95%: {'PASS' if summary['cost_isolation']['pass'] else 'FAIL'}.",
        "",
        "## Compute cost",
        "",
        f"- Fixed-time median nodes: material {nodes['material']:,}; cost-only {nodes['candidate-cost-only']:,}; "
        f"candidate {nodes['candidate']:,}; teacher {nodes['teacher']:,}.",
        f"- Per-position cost-only/material node ratio median {ratio_item['median']:.3f} "
        f"(range {ratio_item['min']:.3f}-{ratio_item['max']:.3f}); <=0.75 in "
        f"{ratio_item['positions_at_or_below_threshold']}/{summary['validation']['positions']} positions: {'PASS' if ratio_item['pass'] else 'FAIL'}.",
        "",
        "## Evaluator content",
        "",
        f"- Candidate versus material bestmove disagreement: {candidate_content['bestmove_disagreements']}/{summary['validation']['positions']}.",
        f"- Candidate versus material >=300cp difference: {candidate_content['large_score_differences_ge_300cp']}/"
        f"{candidate_content['normal_cp_positions']} ordinary-cp positions; mate-like positions excluded: "
        f"{candidate_content['mate_like_positions_excluded_from_cp_metrics']}.",
        f"- Candidate actual-move regret >=300cp: {regret['all']['candidate']['major_regret_ge_300cp']}/"
        f"{regret['all']['candidate']['normal_cp_positions']} ordinary-cp positions.",
        "",
        "## Conclusion",
        "",
        f"- {conclusion['interpretation']}",
        "- The candidate remains rejected; Q20 remains unauthorized.",
        f"- Next: {conclusion['next_action']}",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--measurements", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    try:
        summary = summarize(
            read_json(args.corpus), read_json(args.preregistration), read_json(args.manifest),
            read_jsonl(args.measurements), args.preregistration, args.measurements,
        )
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    summary["artifacts"] = {
        "corpus": {"path": str(args.corpus), "sha256": sha256(args.corpus)},
        "preregistration": {"path": str(args.preregistration), "sha256": sha256(args.preregistration)},
        "manifest": {"path": str(args.manifest), "sha256": sha256(args.manifest)},
        "measurements": {"path": str(args.measurements), "sha256": sha256(args.measurements)},
        "summarizer_source": {"path": str(Path(__file__).resolve()), "sha256": sha256(Path(__file__).resolve())},
    }
    args.summary.parent.mkdir(parents=True, exist_ok=True)
    args.summary.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(report_markdown(summary), encoding="utf-8")
    print(json.dumps({
        "valid": summary["validation"]["valid"],
        "cost_isolated": summary["conclusion"]["cost_isolated"],
        "cost_pressure": summary["conclusion"]["cost_pressure"],
        "content_difference": summary["conclusion"]["content_difference"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
