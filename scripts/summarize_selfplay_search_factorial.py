#!/usr/bin/env python3
"""Summarize C5c3 while preserving its non-strength boundary."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def complete(row: dict) -> bool:
    return row.get("completed_iteration_valid") is True and row.get("completed_bound") == "exact" and row.get("pv_legal") is True


def compression_stable(material: dict[str, dict], nnue: dict[str, dict]) -> bool:
    return all(
        complete(material[name]) and complete(nnue[name])
        and abs(material[name]["score_cp"]) >= 1_000
        and abs(nnue[name]["score_cp"]) <= 100
        for name in ("cold_nmp_on", "warm_nmp_on", "cold_nmp_off")
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = json.loads(args.input.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.selfplay-search-factorial.v1" or document.get("diagnostic_only") is not True:
        raise SystemExit(f"{args.input}: unsupported factorial diagnostic")
    rows, stable = [], []
    for row in document.get("results", []):
        material, nnue = row["results"]["material"], row["results"]["nnue_gate0_init_fix_absolute"]
        all_complete = all(complete(result) for evaluator in (material, nnue) for result in evaluator.values())
        row_summary = {
            "id": row["id"], "categories": row["source"]["categories"], "all_complete": all_complete,
            "cold_to_warm": {name: {"score_changed": evaluator["cold_nmp_on"]["score_cp"] != evaluator["warm_nmp_on"]["score_cp"], "bestmove_changed": evaluator["cold_nmp_on"]["bestmove"] != evaluator["warm_nmp_on"]["bestmove"]} for name, evaluator in (("material", material), ("nnue", nnue))},
            "cold_to_nmp_off": {name: {"score_changed": evaluator["cold_nmp_on"]["score_cp"] != evaluator["cold_nmp_off"]["score_cp"], "bestmove_changed": evaluator["cold_nmp_on"]["bestmove"] != evaluator["cold_nmp_off"]["bestmove"]} for name, evaluator in (("material", material), ("nnue", nnue))},
            "stable_material_loss_nnue_compression": "material_loss" in row["source"]["categories"] and compression_stable(material, nnue),
        }
        if row_summary["stable_material_loss_nnue_compression"]:
            stable.append({"id": row["id"], "categories": row["source"]["categories"], "source": row["source"], "cold_scores": {"material": material["cold_nmp_on"]["score_cp"], "nnue": nnue["cold_nmp_on"]["score_cp"]}, "bestmoves": {"material": material["cold_nmp_on"]["bestmove"], "nnue": nnue["cold_nmp_on"]["bestmove"]}})
        rows.append(row_summary)
    output = {
        "schema": "sekirei.selfplay-search-factorial-summary.v1", "diagnostic_only": True,
        "input": str(args.input),
        "summary": {"positions": len(rows), "all_complete": sum(row["all_complete"] for row in rows), "stable_material_loss_nnue_compression": len(stable), "interpretation": "The score-calibration signal survives these TT/NMP toggles in listed rows. It does not establish that material is correct, that NNUE is wrong, or that either is stronger."},
        "rows": rows, "stable_compression_rows": stable,
        "independent_validation_before_change": {
            "corpus": "13 held-out representative games across six opening groups from the healthy self-play review; exclude all 12 C5c2 positions and all 32 C5c3 positions",
            "contract": "same engine revision, absolute NNUE output, Threads=1, SpecTopN=0, fixed 100k nodes, cold TT",
            "acceptance": "pre-register any calibration metric and require no legal-move/PV/history regression; evaluator diagnostics alone remain insufficient for candidate adoption",
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(output["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
