#!/usr/bin/env python3
"""Select exactly one next-test hypothesis from a budget comparison."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-budget-comparison.v1"


def select(document: dict) -> dict:
    if document.get("schema") != SCHEMA or document.get("diagnostic_only") is not True:
        raise ValueError("input is not a diagnostic-only budget comparison")
    summary = document.get("summary")
    if not isinstance(summary, dict):
        raise ValueError("summary is missing")
    total = summary.get("total")
    comparable = summary.get("comparable")
    changes = summary.get("nonzero_score_changes")
    if not all(isinstance(value, int) and value >= 0 for value in (total, comparable, changes)):
        raise ValueError("summary counts are invalid")
    if comparable == 0:
        selected = "measurement_completion"
        rationale = "完了探索がないため、原因仮説より先に予算と完了状態を固定する。"
        expected = "同一局面・同一条件で比較可能行を増やす。"
        reject = "比較可能行が確保されても予算差が観測されない。"
        next_test = "同一条件の小規模予算段階比較を再実行する。"
    elif changes > 0:
        selected = "search_budget_sensitivity"
        rationale = "完了探索でscore差が観測され、TTによるbestmove変化は別pilotで観測されていない。"
        expected = "同一局面・同一評価器で予算差を縮めるとscore差が減少し、bestmoveは安定する。"
        reject = "同一深さ・同一予算で評価器以外を固定してもscore差が再現しない、または独立hold-outで差が出ない。"
        next_test = "同一4局面の固定depth・同一評価器で、予算段階とPV/boundを保存して再比較する。"
    else:
        selected = "no_observed_budget_effect"
        rationale = "比較可能な完了探索で予算差によるscore変化が観測されなかった。"
        expected = "独立局面・別予算でもbestmoveとscoreが安定する。"
        reject = "独立hold-outまたは別予算で再現可能な差が出る。"
        next_test = "原因変更前に独立hold-outで同じ予算比較を行う。"
    return {
        "schema": "sekirei.floodgate-hypothesis-selection.v1",
        "diagnostic_only": True,
        "source_schema": SCHEMA,
        "source_summary": {"total": total, "comparable": comparable, "nonzero_score_changes": changes},
        "selected": {"id": selected, "rationale": rationale, "expected_change": expected,
                      "rejection_condition": reject, "next_test": next_test},
        "claims": {"strength": "not_permitted", "causal_inference": "not_proven",
                   "implementation_adoption": "not_yet"},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = select(json.loads(args.input.read_text(encoding="utf-8")))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"selected hypothesis: {result['selected']['id']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
