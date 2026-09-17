#!/usr/bin/env python3
"""Classify the pre-registered C5c2 evaluator diagnostic without Elo claims."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def complete(result: dict) -> bool:
    return result.get("completed_iteration_valid") is True and result.get("completed_bound") == "exact" and result.get("pv_legal") is True


def compressed(material: dict, nnue: dict) -> bool:
    return abs(material["score_cp"]) >= 1_000 and abs(nnue["score_cp"]) <= 100


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = json.loads(args.input.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.fixed-selfplay-evaluator-diagnostic.v1" or document.get("diagnostic_only") is not True:
        raise SystemExit(f"{args.input}: unsupported diagnostic")
    rows = []
    for row in document.get("results", []):
        category = row["source"]["selection_category"]
        for mode in ("free", "actual_root"):
            material = row["results"]["material"][mode]
            nnue = row["results"]["nnue_gate0_init_fix_absolute"][mode]
            rows.append({"id": row["id"], "category": category, "mode": mode, "comparable": complete(material) and complete(nnue), "compressed": complete(material) and complete(nnue) and compressed(material, nnue), "material_score_cp": material["score_cp"], "nnue_score_cp": nnue["score_cp"], "score_delta_cp": nnue["score_cp"] - material["score_cp"], "nnue_bestmove_matches_actual": nnue["bestmove"] == row["actual_move_usi"]})
    material_rows = [row for row in rows if row["category"] == "material_loss"]
    free_compressed = sum(row["compressed"] for row in material_rows if row["mode"] == "free")
    actual_compressed = sum(row["compressed"] for row in material_rows if row["mode"] == "actual_root")
    trigger = free_compressed >= 3 and actual_compressed >= 3
    output = {
        "schema": "sekirei.fixed-selfplay-evaluator-classification.v1", "diagnostic_only": True,
        "input": str(args.input),
        "rule": {"name": "material_loss_nnue_compression", "material_abs_cp_min": 1000, "nnue_abs_cp_max": 100, "reproducible_if": "at least three completed material_loss rows in both free and actual_root modes"},
        "rows": rows,
        "summary": {"rows": len(rows), "comparable": sum(row["comparable"] for row in rows), "incomplete": sum(not row["comparable"] for row in rows), "material_loss_free_compressed": free_compressed, "material_loss_actual_root_compressed": actual_compressed, "c5c3_trigger": trigger, "interpretation": "score calibration hypothesis only; strength is unmeasured"},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(output["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
