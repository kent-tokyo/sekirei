#!/usr/bin/env python3
"""Run the preregistered Q21g B/T/M teacher-judgment audit.

This is fixed-node diagnostic evidence, not a strength test.  Every search is
a fresh process (cold TT).  Only the last completed exact iterative-deepening
result is used for score metrics; incomplete, mate, observed, and newly
reanalyzed values remain separate in the JSON report.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import statistics
import subprocess
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from run_fixed_selfplay_diagnostic import parse_fields


MATE_THRESHOLD_CP = 899_000
MAJOR_BLUNDER_CP = 300
EVALUATORS = ("B", "T", "M")
KNOWN_FIXTURES = (
    {
        "id": "known-mate-in-one",
        "sfen": "k8/2K6/9/9/4R4/9/9/9/9 b - 1",
        "expected_move": "5e9e",
        "expected": "mate_in_one",
        "source": "crates/sekirei-core/src/search.rs MATE_IN_1_SFEN",
    },
    {
        "id": "known-hanging-rook-capture",
        "sfen": "4k4/9/9/4r4/9/9/4R4/9/4K4 b - 1",
        "expected_move": "5g5d",
        "expected": "capture_unprotected_rook",
        "source": "Q21g hand-verified legal material-loss-avoidance fixture",
    },
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def score_kind(score: int | None) -> str:
    if score is None:
        return "missing"
    return "mate" if abs(score) >= MATE_THRESHOLD_CP else "cp"


def exact(result: dict[str, Any]) -> bool:
    return (
        result.get("completed_iteration_valid") is True
        and result.get("completed_bound") == "exact"
        and result.get("pv_legal") is True
        and result.get("history_matches_expected") is True
    )


def evaluator_args(
    name: str, baseline: Path, teacher: Path
) -> tuple[list[str], dict[str, Any]]:
    if name == "B":
        return ["--weights", str(baseline), "--nnue-output", "absolute"], {
            "kind": "public_baseline",
            "weights": str(baseline),
            "weights_sha256": sha256(baseline),
            "nnue_output": "absolute",
        }
    if name == "T":
        return ["--weights", str(teacher), "--nnue-output", "residual-material"], {
            "kind": "fixed_teacher_candidate",
            "weights": str(teacher),
            "weights_sha256": sha256(teacher),
            "nnue_output": "residual-material",
        }
    if name == "M":
        return [], {"kind": "material", "weights": None, "nnue_output": None}
    raise ValueError(f"unknown evaluator {name}")


def probe_scores(
    probe: Path, weights: Path, output_mode: str, sfens: list[str]
) -> list[int]:
    command = [str(probe), str(weights), "--json", "--nnue-output", output_mode]
    for sfen in sfens:
        command.extend(("--sfen", sfen))
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=180)
    if completed.returncode:
        raise RuntimeError(f"NNUE probe failed: {completed.stderr.strip()[-1200:]}")
    document = json.loads(completed.stdout)
    probes = document.get("probes")
    if not isinstance(probes, list) or len(probes) != len(sfens):
        raise ValueError("NNUE probe did not return one score per SFEN")
    scores = [row.get("score_cp") for row in probes]
    if any(not isinstance(score, int) or isinstance(score, bool) for score in scores):
        raise ValueError("NNUE probe returned a non-integer score")
    return scores


def run_search(
    engine: Path,
    row: dict[str, Any],
    nodes: int,
    name: str,
    baseline: Path,
    teacher: Path,
    root_move: str | None = None,
) -> dict[str, Any]:
    position = row["position"]
    command = [str(engine), "--nodes", str(nodes), "--sfen", position["initial_sfen"]]
    history = position.get("history_before_usi", [])
    command.extend(("--moves", " ".join(history)))
    command.extend(("--expected-sfen", position["sfen"]))
    evaluator_cli, _ = evaluator_args(name, baseline, teacher)
    command.extend(evaluator_cli)
    if root_move is not None:
        command.extend(("--root-move", root_move))
    environment = dict(os.environ)
    environment["RAYON_NUM_THREADS"] = "1"
    completed = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=300,
        env=environment,
    )
    if completed.returncode:
        raise RuntimeError(
            f"search failed for {row['id']} {name} {nodes}: {completed.stderr.strip()[-1200:]}"
        )
    fields = parse_fields(completed.stdout)
    score = int(fields["score_cp"])
    result = {
        "budget_nodes": nodes,
        "root_move": root_move,
        "bestmove": fields["bestmove"],
        "depth": int(fields["depth"]),
        "score_cp": score,
        "score_kind": score_kind(score),
        "nodes": int(fields["nodes"]),
        "elapsed_ms": int(fields["elapsed_ms"]),
        "bound": fields.get("bound"),
        "completed_bound": fields.get("completed_bound"),
        "completed_iteration_valid": fields["completed_iteration_valid"] == "true",
        "aborted": fields.get("aborted") == "true",
        "abort_reason": fields.get("abort_reason"),
        "pv_usi": fields["pv_usi"].split(",") if fields["pv_usi"] else [],
        "pv_legal": fields["pv_legal"] == "true",
        "history_moves": int(fields.get("history_moves", 0)),
        "history_replayed": fields.get("history_replayed") == "true",
        "history_matches_expected": fields.get("history_matches_expected") == "true",
    }
    result["usable_exact_score"] = exact(result)
    return result


def fixture_row(fixture: dict[str, str]) -> dict[str, Any]:
    return {
        "id": fixture["id"],
        "position": {
            "initial_sfen": fixture["sfen"],
            "history_before_usi": [],
            "sfen": fixture["sfen"],
        },
    }


def deep_selection_key(row: dict[str, Any], shallow_nodes: int = 20_000) -> tuple[Any, ...]:
    results = row["search"][str(shallow_nodes)]
    t, m = results["T"]["free"], results["M"]["free"]
    any_inexact = any(not exact(results[name]["free"]) for name in EVALUATORS)
    bestmove_disagrees = t.get("bestmove") != m.get("bestmove")
    sign_disagrees = (
        t.get("score_kind") == m.get("score_kind") == "cp"
        and (t["score_cp"] > 0) != (m["score_cp"] > 0)
    )
    delta = abs(t.get("score_cp", 0) - m.get("score_cp", 0))
    return (
        0 if any_inexact else 1,
        0 if bestmove_disagrees else 1,
        0 if sign_disagrees else 1,
        -delta,
        row["id"],
    )


def select_deep(rows: list[dict[str, Any]], limit: int, shallow_nodes: int = 20_000) -> list[str]:
    ranked = sorted(rows, key=lambda row: deep_selection_key(row, shallow_nodes))
    selected: list[dict[str, Any]] = []
    used: set[str] = set()
    for row in ranked:
        stratum = f"{row['attributes']['phase']}:{row['attributes']['material_band']}"
        if stratum not in used:
            selected.append(row)
            used.add(stratum)
        if len(selected) == limit:
            break
    for row in ranked:
        if len(selected) == limit:
            break
        if row not in selected:
            selected.append(row)
    return [row["id"] for row in selected]


def pair_metrics(
    rows: list[dict[str, Any]], left: str, right: str, shallow_nodes: int = 20_000
) -> dict[str, Any]:
    comparable = []
    excluded = Counter()
    for row in rows:
        if row["selection_class"] != "normal":
            continue
        results = row["search"][str(shallow_nodes)]
        a, b = results[left]["free"], results[right]["free"]
        if not exact(a) or not exact(b):
            excluded["incomplete_or_nonexact"] += 1
        elif a["score_kind"] != "cp" or b["score_kind"] != "cp":
            excluded["mate_or_mixed_score"] += 1
        else:
            comparable.append((row, a["score_cp"], b["score_cp"]))

    def summarize(items: list[tuple[dict[str, Any], int, int]]) -> dict[str, Any]:
        differences = [abs(a - b) for _, a, b in items]
        return {
            "positions": len(items),
            "mae_cp": statistics.fmean(differences) if differences else None,
            "sign_mismatches": sum((a > 0) != (b > 0) for _, a, b in items),
            "bestmove_mismatches": sum(
                row["search"][str(shallow_nodes)][left]["free"]["bestmove"]
                != row["search"][str(shallow_nodes)][right]["free"]["bestmove"]
                for row, _, _ in items
            ),
        }

    by_stratum: dict[str, list[tuple[dict[str, Any], int, int]]] = defaultdict(list)
    for item in comparable:
        row = item[0]
        key = f"{row['attributes']['phase']}:{row['attributes']['material_band']}"
        by_stratum[key].append(item)
    return {
        "left": left,
        "right_reference_estimate": right,
        "reference_is_ground_truth": False,
        "overall": summarize(comparable),
        "excluded_normal_positions": dict(sorted(excluded.items())),
        "by_phase_material": {
            key: summarize(items) for key, items in sorted(by_stratum.items())
        },
    }


def deep_metrics(
    rows: list[dict[str, Any]],
    selected_ids: set[str],
    shallow_nodes: int = 20_000,
    deep_nodes: int = 100_000,
    final_nodes: int = 1_000_000,
) -> dict[str, Any]:
    evaluator_summary: dict[str, dict[str, Any]] = {}
    major_blunders: list[dict[str, Any]] = []
    mate_cases: list[dict[str, Any]] = []
    for name in EVALUATORS:
        stable, comparable, losses = 0, 0, []
        incomplete = 0
        mate = 0
        for row in rows:
            if row["id"] not in selected_ids:
                continue
            r20 = row["search"][str(shallow_nodes)][name]["free"]
            r100 = row["search"][str(deep_nodes)][name]["free"]
            r1m = row["search"][str(final_nodes)][name]["free"]
            chosen_key = f"chosen20:{r20['bestmove']}"
            fixed = row["search"][str(final_nodes)][name].get(chosen_key)
            if not all(exact(result) for result in (r20, r100, r1m)) or fixed is None or not exact(fixed):
                incomplete += 1
                continue
            if r1m["score_kind"] == "mate" or fixed["score_kind"] == "mate":
                mate += 1
                mate_cases.append(
                    {
                        "position_id": row["id"],
                        "evaluator": name,
                        "chosen20": r20["bestmove"],
                        "reference1m": r1m["bestmove"],
                        "free1m_score_cp": r1m["score_cp"],
                        "chosen20_fixed1m_score_cp": fixed["score_cp"],
                    }
                )
                continue
            comparable += 1
            stable += r20["bestmove"] == r100["bestmove"] == r1m["bestmove"]
            loss = r1m["score_cp"] - fixed["score_cp"]
            losses.append(loss)
            if loss >= MAJOR_BLUNDER_CP:
                major_blunders.append(
                    {
                        "position_id": row["id"],
                        "evaluator": name,
                        "phase": row["attributes"]["phase"],
                        "material_band": row["attributes"]["material_band"],
                        "chosen20": r20["bestmove"],
                        "reference1m": r1m["bestmove"],
                        "rank_loss_cp": loss,
                    }
                )
        evaluator_summary[name] = {
            "selected_positions": len(selected_ids),
            "comparable_cp_positions": comparable,
            "incomplete_or_nonexact_positions": incomplete,
            "mate_or_mixed_positions": mate,
            "rank_stable_20k_100k_1m": stable,
            "rank_loss_mean_cp": statistics.fmean(losses) if losses else None,
            "rank_loss_median_cp": statistics.median(losses) if losses else None,
            "major_blunders_ge_300cp": sum(
                item["evaluator"] == name for item in major_blunders
            ),
        }
    disagreements = []
    for row in rows:
        if row["id"] not in selected_ids:
            continue
        t = row["search"][str(final_nodes)]["T"]["free"]
        m = row["search"][str(final_nodes)]["M"]["free"]
        if exact(t) and exact(m) and t["bestmove"] != m["bestmove"]:
            disagreements.append(
                {
                    "position_id": row["id"],
                    "T_bestmove": t["bestmove"],
                    "M_bestmove": m["bestmove"],
                    "T_score_cp": t["score_cp"],
                    "M_score_cp": m["score_cp"],
                    "status": "unresolved_reference_disagreement",
                }
            )
    return {
        "major_blunder_threshold_cp": MAJOR_BLUNDER_CP,
        "candidate_ranking": evaluator_summary,
        "major_blunders": major_blunders,
        "mate_cases_separate": mate_cases,
        "teacher_material_1m_disagreements": disagreements,
    }


def resource_proxy(rows: list[dict[str, Any]], shallow_nodes: int = 20_000) -> dict[str, Any]:
    elapsed: dict[str, list[int]] = defaultdict(list)
    for row in rows:
        for name in EVALUATORS:
            result = row["search"][str(shallow_nodes)][name]["free"]
            if exact(result):
                elapsed[name].append(result["elapsed_ms"])
    medians = {
        name: statistics.median(values) if values else None
        for name, values in elapsed.items()
    }
    material = medians.get("M")
    ratios = {
        name: (value / material if value is not None and material not in (None, 0) else None)
        for name, value in medians.items()
    }
    return {
        "fixed_nodes": shallow_nodes,
        "median_elapsed_ms": medians,
        "elapsed_ratio_vs_M": ratios,
        "same_time_result": "unmeasured",
        "interpretation": "fixed-node elapsed is a cost proxy, not a same-time strength result",
    }


def classify(
    rows: list[dict[str, Any]],
    fixtures: list[dict[str, Any]],
    deep: dict[str, Any],
    cost_proxy: dict[str, Any],
    deterministic_rerun: dict[str, Any],
) -> dict[str, Any]:
    state_failures = []
    for row in rows:
        for budget, evaluators in row["search"].items():
            for name, modes in evaluators.items():
                for mode, result in modes.items():
                    if not result.get("history_matches_expected") or not result.get("pv_legal"):
                        state_failures.append(f"{row['id']}:{budget}:{name}:{mode}")
    teacher_fixture_failures = []
    for fixture in fixtures:
        for budget, run in fixture["runs"]["T"].items():
            if not exact(run) or run["bestmove"] != fixture["expected_move"]:
                teacher_fixture_failures.append(f"{fixture['id']}:{budget}")
    disagreements = len(deep["teacher_material_1m_disagreements"])
    teacher_major = deep["candidate_ranking"]["T"]["major_blunders_ge_300cp"]
    teacher_cost_ratio = cost_proxy["elapsed_ratio_vs_M"].get("T")
    search_volume_supported = (
        teacher_major > 0
        and isinstance(teacher_cost_ratio, (int, float))
        and teacher_cost_ratio > 1.0
    )
    if not deterministic_rerun["pass"]:
        next_factor = "search_determinism"
        conclusion = "state_inconsistency"
        state_failures.append("final-node deterministic rerun mismatch")
    elif state_failures:
        next_factor = "state_correctness"
        conclusion = "state_inconsistency"
    elif teacher_fixture_failures:
        next_factor = "teacher_search_or_labels"
        conclusion = "teacher_judgment_deficiency"
    elif search_volume_supported:
        next_factor = "same_time_nnue_cost"
        conclusion = "same_time_search_volume_deficiency"
    elif disagreements:
        next_factor = "teacher_reference_quality"
        conclusion = "unresolved"
    elif teacher_major:
        next_factor = "teacher_search_budget"
        conclusion = "teacher_search_volume_deficiency"
    else:
        next_factor = "same_time_nnue_cost"
        conclusion = "same_time_search_volume_requires_measurement"
    return {
        "teacher_judgment_deficiency": {
            "status": "supported" if teacher_fixture_failures else "not_proven",
            "known_fixture_failures": teacher_fixture_failures,
        },
        "student_S_imitation_deficiency": {
            "status": "unmeasured",
            "reason": "no student S was preregistered",
        },
        "same_time_search_volume_deficiency": {
            "status": "supported_by_cost_and_budget_instability" if search_volume_supported else "unmeasured",
            "teacher_20k_major_blunders_vs_teacher_1m": teacher_major,
            "teacher_elapsed_ratio_vs_M_at_fixed_nodes": teacher_cost_ratio,
            "caveat": "fixed-node elapsed plus rank instability diagnose risk; they are not a same-time match result",
        },
        "state_inconsistency": {"status": "detected" if state_failures else "not_detected", "failures": state_failures},
        "unresolved": {"status": "present" if disagreements else "none_observed", "deep_T_M_disagreements": disagreements},
        "conclusion": conclusion,
        "next_single_factor": next_factor,
        "same_labels_scale_up_allowed": False,
        "external_teacher_started": False,
    }


def preregistration(args: argparse.Namespace, corpus: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema": "sekirei.q21g-preregistration.v1",
        "corpus_sha256": sha256(args.corpus),
        "positions": len(corpus["entries"]),
        "evaluators": {
            "B": evaluator_args("B", args.baseline, args.teacher)[1],
            "T": evaluator_args("T", args.baseline, args.teacher)[1],
            "M": evaluator_args("M", args.baseline, args.teacher)[1],
            "S": {"status": "unmeasured", "reason": "not selected"},
        },
        "budgets_nodes": [args.shallow_nodes, args.deep_nodes, args.final_nodes],
        "deep_limit": args.deep_limit,
        "deep_selection_rule": [
            "inexact 20k result first",
            "T/M 20k bestmove disagreement",
            "T/M 20k cp sign disagreement",
            "descending absolute T-M score gap",
            "cover phase/material strata before filling by rank",
            "position id tie-break",
        ],
        "modes": {
            "20k": ["free", "actual_root"],
            "100k_and_1m": ["free", "actual_root", "20k_chosen_root"],
        },
        "determinism_smoke": "first selected deep position, T at final budget, three fresh-process reruns",
        "score_contract": {
            "usable": "completed_iteration_valid && completed_bound=exact && pv_legal && history_matches_expected",
            "mate_threshold_cp": MATE_THRESHOLD_CP,
            "major_blunder_cp": MAJOR_BLUNDER_CP,
            "observed_and_prior_reanalysis": "preserved in corpus, never substituted for new search",
            "teacher_deep_result": "reference estimate, not ground truth",
        },
        "tt": "cold fresh process per search",
        "threads": 1,
        "thread_control": "RAYON_NUM_THREADS=1 for every search subprocess",
        "spec_top_n": 0,
        "strength_claim": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--shallow-nodes", type=int, default=20_000)
    parser.add_argument("--deep-nodes", type=int, default=100_000)
    parser.add_argument("--final-nodes", type=int, default=1_000_000)
    parser.add_argument("--deep-limit", type=int, default=14)
    args = parser.parse_args()
    if not (0 < args.shallow_nodes < args.deep_nodes < args.final_nodes):
        parser.error("node budgets must be positive and strictly increasing")
    if not 0 < args.deep_limit <= 14:
        parser.error("deep limit must be in 1..=14")
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    if corpus.get("schema") != "sekirei.q21g-teacher-corpus.v1":
        parser.error("invalid Q21g corpus schema")
    rows = json.loads(json.dumps(corpus["entries"]))
    args.output_dir.mkdir(parents=True, exist_ok=True)
    prereg = preregistration(args, corpus)
    prereg_path = args.output_dir / "preregistration.json"
    prereg_path.write_text(json.dumps(prereg, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    sfens = [row["position"]["sfen"] for row in rows]
    static_b = probe_scores(args.probe, args.baseline, "absolute", sfens)
    static_t = probe_scores(args.probe, args.teacher, "residual-material", sfens)
    for row, b_score, t_score in zip(rows, static_b, static_t, strict=True):
        row["static_evaluation"] = {
            "B": {"score_cp": b_score, "score_kind": score_kind(b_score)},
            "T": {"score_cp": t_score, "score_kind": score_kind(t_score)},
            "M": {"score_cp": row["attributes"]["material_stm_cp"], "score_kind": "cp"},
            "S": {"status": "unmeasured"},
        }
        row["search"] = {str(args.shallow_nodes): {}}
        for name in EVALUATORS:
            row["search"][str(args.shallow_nodes)][name] = {
                "free": run_search(args.engine, row, args.shallow_nodes, name, args.baseline, args.teacher),
                "actual_root": run_search(
                    args.engine,
                    row,
                    args.shallow_nodes,
                    name,
                    args.baseline,
                    args.teacher,
                    row["position"]["actual_move_usi"],
                ),
            }

    selected_ids = set(select_deep(rows, args.deep_limit, args.shallow_nodes))
    deep_selection = [
        {
            "id": row["id"],
            "phase": row["attributes"]["phase"],
            "material_band": row["attributes"]["material_band"],
            "selection_key": list(deep_selection_key(row, args.shallow_nodes)),
        }
        for row in rows
        if row["id"] in selected_ids
    ]
    (args.output_dir / "deep-selection.json").write_text(
        json.dumps({"selected": deep_selection, "limit": args.deep_limit}, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    for row in rows:
        if row["id"] not in selected_ids:
            continue
        for nodes in (args.deep_nodes, args.final_nodes):
            row["search"][str(nodes)] = {}
            for name in EVALUATORS:
                chosen20 = row["search"][str(args.shallow_nodes)][name]["free"]["bestmove"]
                modes = {
                    "free": run_search(args.engine, row, nodes, name, args.baseline, args.teacher),
                    "actual_root": run_search(
                        args.engine, row, nodes, name, args.baseline, args.teacher,
                        row["position"]["actual_move_usi"],
                    ),
                }
                chosen_key = f"chosen20:{chosen20}"
                if chosen20 == row["position"]["actual_move_usi"]:
                    modes[chosen_key] = modes["actual_root"]
                else:
                    modes[chosen_key] = run_search(
                        args.engine, row, nodes, name, args.baseline, args.teacher, chosen20
                    )
                row["search"][str(nodes)][name] = modes

    fixture_results = []
    for fixture in KNOWN_FIXTURES:
        result = {**fixture, "runs": {}}
        row = fixture_row(fixture)
        for name in EVALUATORS:
            result["runs"][name] = {
                str(nodes): run_search(args.engine, row, nodes, name, args.baseline, args.teacher)
                for nodes in (args.shallow_nodes, args.deep_nodes, args.final_nodes)
            }
        fixture_results.append(result)

    deep = deep_metrics(
        rows, selected_ids, args.shallow_nodes, args.deep_nodes, args.final_nodes
    )
    cost_proxy = resource_proxy(rows, args.shallow_nodes)
    smoke_row = min((row for row in rows if row["id"] in selected_ids), key=lambda row: row["id"])
    smoke_runs = [
        run_search(
            args.engine, smoke_row, args.final_nodes, "T", args.baseline, args.teacher
        )
        for _ in range(3)
    ]
    smoke_signatures = [
        {
            key: run[key]
            for key in ("bestmove", "score_cp", "depth", "nodes", "completed_bound", "usable_exact_score")
        }
        for run in smoke_runs
    ]
    deterministic_rerun = {
        "position_id": smoke_row["id"],
        "evaluator": "T",
        "budget_nodes": args.final_nodes,
        "runs": smoke_signatures,
        "pass": all(signature == smoke_signatures[0] for signature in smoke_signatures[1:]),
    }
    report = {
        "schema": "sekirei.q21g-teacher-judgment-audit.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "inputs": {
            "corpus": str(args.corpus),
            "corpus_sha256": sha256(args.corpus),
            "preregistration": str(prereg_path),
            "preregistration_sha256": sha256(prereg_path),
            "engine": str(args.engine),
            "engine_sha256": sha256(args.engine),
            "probe": str(args.probe),
            "probe_sha256": sha256(args.probe),
        },
        "evaluator_identity": prereg["evaluators"],
        "load_acknowledgement": {
            "B": "successful explicit --weights diagnostic and static probe",
            "T": "successful explicit --weights diagnostic and static probe",
            "M": "no EvalFile by contract",
        },
        "student_S": {"status": "unmeasured", "imitation_error": "unmeasured"},
        "known_fixtures": fixture_results,
        "normal_position_metrics": {
            "B_vs_T_reference_estimate": pair_metrics(rows, "B", "T", args.shallow_nodes),
            "M_vs_T_reference_estimate": pair_metrics(rows, "M", "T", args.shallow_nodes),
        },
        "deep_metrics": deep,
        "deterministic_rerun": deterministic_rerun,
        "fixed_node_cost_proxy": cost_proxy,
        "wdl_calibration": {"status": "not_applicable", "reason": "no win-probability output was used"},
        "classification": classify(rows, fixture_results, deep, cost_proxy, deterministic_rerun),
        "rows": rows,
    }
    report_path = args.output_dir / "audit.json"
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    summary = {
        "positions": len(rows),
        "deep_positions": len(selected_ids),
        "known_fixtures": len(fixture_results),
        "classification": report["classification"],
        "normal_position_metrics": report["normal_position_metrics"],
        "deep_metrics": deep,
        "fixed_node_cost_proxy": report["fixed_node_cost_proxy"],
    }
    summary_path = args.output_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
