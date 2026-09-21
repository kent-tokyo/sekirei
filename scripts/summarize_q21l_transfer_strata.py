#!/usr/bin/env python3
"""Stratify Q21k's fixed-position transfer audit without choosing a model.

The input summary is the validated Q21j-compatible content/cost audit.  This
tool joins only position metadata (phase, hand-aware material, forcing class,
and completed fixed-node depth).  It does not relabel positions, train a
checkpoint, or turn the played move into ground truth.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
PROFILE_SPEC = importlib.util.spec_from_file_location(
    "nnue_root_profiles", ROOT / "scripts" / "compare_nnue_root_profiles.py"
)
assert PROFILE_SPEC and PROFILE_SPEC.loader
PROFILES = importlib.util.module_from_spec(PROFILE_SPEC)
PROFILE_SPEC.loader.exec_module(PROFILES)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def median(values: list[int | float]) -> int | float | None:
    if not values:
        return None
    value = statistics.median(values)
    return int(value) if float(value).is_integer() else value


def depth_relation(candidate: int, material: int) -> str:
    if candidate < material:
        return "candidate_shallower"
    if candidate > material:
        return "candidate_deeper"
    return "same_depth"


def king_danger(classification: dict[str, Any]) -> str:
    if classification["in_check"]:
        return "in_check"
    if classification["legal_checks"] > 0:
        return "checking_option_available"
    return "no_immediate_check"


def summarize_rows(rows: list[dict[str, Any]]) -> dict[str, Any]:
    comparable = [row for row in rows if not row["candidate_material_mate_like"]]
    regret_eligible = [row for row in rows if row["candidate_regret_cp"] is not None]
    material_regret_eligible = [row for row in rows if row["material_regret_cp"] is not None]
    differences = [row["candidate_material_abs_score_difference_cp"] for row in comparable]
    candidate_regrets = [row["candidate_regret_cp"] for row in regret_eligible]
    material_regrets = [row["material_regret_cp"] for row in material_regret_eligible]
    return {
        "positions": len(rows),
        "candidate_losses": sum(row["game_result"] == "baseline_win" for row in rows),
        "candidate_wins": sum(row["game_result"] == "candidate_win" for row in rows),
        "draws": sum(row["game_result"] == "draw" for row in rows),
        "bestmove_disagreements": sum(row["bestmove_disagrees"] for row in rows),
        "bestmove_disagreement_rate": (
            sum(row["bestmove_disagrees"] for row in rows) / len(rows) if rows else None
        ),
        "ordinary_cp_positions": len(comparable),
        "large_score_differences_ge_300cp": sum(
            row["large_score_difference"] for row in comparable
        ),
        "median_abs_score_difference_cp": median(differences),
        "candidate_major_actual_move_regret_ge_300cp": sum(
            value >= 300 for value in candidate_regrets
        ),
        "candidate_regret_eligible_positions": len(regret_eligible),
        "candidate_median_actual_move_regret_cp": median(candidate_regrets),
        "material_major_actual_move_regret_ge_300cp": sum(
            value >= 300 for value in material_regrets
        ),
        "material_regret_eligible_positions": len(material_regret_eligible),
        "material_median_actual_move_regret_cp": median(material_regrets),
        "candidate_median_depth": median([row["candidate_depth"] for row in rows]),
        "material_median_depth": median([row["material_depth"] for row in rows]),
        "candidate_shallower_positions": sum(
            row["depth_relation"] == "candidate_shallower" for row in rows
        ),
    }


def build(corpus: dict[str, Any], summary: dict[str, Any], forcing: dict[str, Any]) -> dict[str, Any]:
    if corpus.get("schema") != "sekirei.q21j-failure-audit-corpus.v1":
        raise ValueError("unsupported failure-audit corpus")
    if summary.get("schema") != "sekirei.q21j-failure-audit-summary.v1":
        raise ValueError("unsupported failure-audit summary")
    if forcing.get("schema") != "sekirei.forcing-position-classification.v1":
        raise ValueError("unsupported forcing classification")
    positions = {item["id"]: item for item in corpus["positions"]}
    classifications = {item["id"]: item for item in forcing["entries"]}
    diagnostics = {item["position_id"]: item for item in summary["positions"]}
    ids = set(positions)
    if len(ids) != 32 or set(classifications) != ids or set(diagnostics) != ids:
        raise ValueError("corpus, forcing classification, and diagnostic IDs differ")

    rows = []
    for position_id in sorted(ids):
        source = positions[position_id]
        diagnostic = diagnostics[position_id]
        classification = classifications[position_id]
        attributes = PROFILES.attributes(source["position"]["sfen"])
        candidate = diagnostic["fixed_nodes"]["candidate"]
        material = diagnostic["fixed_nodes"]["material"]
        candidate_regret = diagnostic["regret"]["candidate"]
        material_regret = diagnostic["regret"]["material"]
        rows.append({
            "position_id": position_id,
            "game_num": source["game_num"],
            "game_result": source["game_result"],
            "selection_rule": source["selection"]["rule"],
            "phase": attributes["phase"],
            "move_number": attributes["move_number"],
            "material_band": attributes["material_band"],
            "material_stm_cp": attributes["material_stm_cp"],
            "forcing_class": classification["forcing_class"],
            "in_check": classification["in_check"],
            "legal_moves": classification["legal_moves"],
            "legal_captures": classification["legal_captures"],
            "legal_checks": classification["legal_checks"],
            "king_danger": king_danger(classification),
            "candidate_depth": candidate["depth"],
            "material_depth": material["depth"],
            "depth_relation": depth_relation(candidate["depth"], material["depth"]),
            "bestmove_disagrees": diagnostic["candidate_material"]["bestmove_disagrees"],
            "candidate_material_mate_like": diagnostic["candidate_material"]["mate_like"],
            "candidate_material_abs_score_difference_cp": diagnostic["candidate_material"][
                "absolute_score_difference_cp"
            ],
            "large_score_difference": diagnostic["candidate_material"]["large_score_difference"],
            "candidate_regret_cp": candidate_regret["cp"],
            "candidate_actual_move": candidate_regret["actual_move"],
            "candidate_free_bestmove": candidate_regret["free_bestmove"],
            "material_regret_cp": material_regret["cp"],
            "material_free_bestmove": material_regret["free_bestmove"],
            "cost_only_material_node_ratio": diagnostic["cost_only_material_node_ratio"],
        })

    dimensions = (
        "game_result",
        "phase",
        "material_band",
        "forcing_class",
        "king_danger",
        "depth_relation",
    )
    stratified: dict[str, dict[str, Any]] = {}
    for dimension in dimensions:
        groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
        for row in rows:
            groups[str(row[dimension])].append(row)
        stratified[dimension] = {
            value: summarize_rows(items) for value, items in sorted(groups.items())
        }
    return {
        "schema": "sekirei.q21l-transfer-strata.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "label_boundary": (
            "played moves remain observations; candidate and material are compared only under the "
            "same fixed-node re-search contract"
        ),
        "overall": summarize_rows(rows),
        "stratified": stratified,
        "rows": rows,
    }


def report(document: dict[str, Any]) -> str:
    overall = document["overall"]
    lines = [
        "# Q21l transfer audit",
        "",
        "Diagnostic only. Played moves are observations, not labels.",
        "",
        "## Overall",
        "",
        f"- Candidate/material bestmove disagreement: {overall['bestmove_disagreements']}/{overall['positions']}.",
        f"- Candidate major actual-move regret: {overall['candidate_major_actual_move_regret_ge_300cp']}/{overall['candidate_regret_eligible_positions']}; material: {overall['material_major_actual_move_regret_ge_300cp']}/{overall['material_regret_eligible_positions']}.",
        f"- Candidate was shallower than material in {overall['candidate_shallower_positions']}/{overall['positions']} fixed-node positions.",
        "",
        "## Strata",
        "",
    ]
    for dimension, groups in document["stratified"].items():
        lines.append(f"### {dimension}")
        lines.append("")
        for value, item in groups.items():
            lines.append(
                f"- {value}: n={item['positions']}, mismatch={item['bestmove_disagreements']}, "
                f"candidate major regret={item['candidate_major_actual_move_regret_ge_300cp']}/"
                f"{item['candidate_regret_eligible_positions']}, material major regret="
                f"{item['material_major_actual_move_regret_ge_300cp']}/"
                f"{item['material_regret_eligible_positions']}, depths="
                f"{item['candidate_median_depth']}/{item['material_median_depth']}."
            )
        lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--forcing", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    document = build(load(args.corpus), load(args.summary), load(args.forcing))
    document["artifacts"] = {
        "corpus": {"path": str(args.corpus), "sha256": sha256(args.corpus)},
        "summary": {"path": str(args.summary), "sha256": sha256(args.summary)},
        "forcing": {"path": str(args.forcing), "sha256": sha256(args.forcing)},
        "summarizer": {"path": str(Path(__file__).resolve()), "sha256": sha256(Path(__file__).resolve())},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    args.report.write_text(report(document), encoding="utf-8")
    print(json.dumps(document["overall"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
