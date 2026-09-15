#!/usr/bin/env python3
"""Build a non-causal hypothesis table from a diagnostic summary."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from validate_floodgate_diagnostic_summary import validate


HYPOTHESES = {
    "incomplete_search": {
        "hypothesis": "探索不足または時間/abort境界",
        "support": "同一条件で予算を増やすとexact boundと安定PVが得られる",
        "reject": "十分な固定予算でexact boundでも差が再現しない",
        "next_test": "同一局面・同一条件でnode budgetを段階拡大する",
    },
    "root_score_gap_observed": {
        "hypothesis": "評価器またはroot制約の影響",
        "support": "同一予算・同一深さで評価器だけを変えて差が再現する",
        "reject": "評価器を固定しても差が再現せず、探索条件差でのみ発生する",
        "next_test": "material/適格NNUEを同一条件で比較する",
    },
    "tt_move_change_observed": {
        "hypothesis": "TT状態または履歴依存の影響",
        "support": "cold/warm条件の反復で同じ局面の差が再現する",
        "reject": "独立プロセスのcold実行でも同じ差が出る",
        "next_test": "TT cold/warmを固定し、history replay状態を併記する",
    },
    "no_difference_observed": {
        "hypothesis": "この予算・局面では差を観測できない",
        "support": "独立局面・複数予算でも差が観測されない",
        "reject": "別予算または独立局面で再現可能な差が出る",
        "next_test": "局面を増やす前に予算と完了状態を確認する",
    },
}


def build(document: dict) -> dict:
    errors = validate(document)
    if errors:
        raise ValueError("invalid diagnostic summary: " + ", ".join(errors))
    counts = {name: 0 for name in HYPOTHESES}
    for row in document["rows"]:
        counts[row["diagnostic_class"]] = counts.get(row["diagnostic_class"], 0) + 1
    rows = []
    for name, definition in HYPOTHESES.items():
        rows.append({"diagnostic_class": name, "observed_rows": counts[name],
                     "status": "unknown", **definition})
    execution = document.get("execution", {})
    source = {
        "source_revision": execution.get("source_revision"),
        "binary": execution.get("binary"),
        "weights": execution.get("weights"),
        "options": execution.get("options"),
        "corpus_sha256": execution.get("corpus_sha256"),
    }
    return {
        "schema": "sekirei.floodgate-diagnostic-hypotheses.v1",
        "diagnostic_only": True,
        "source_schema": document["schema"],
        "source": source,
        "rows": rows,
        "claims": {
            "strength": "not_permitted",
            "causal_inference": "not_proven",
            "played_move_is_label": False,
        },
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        result = build(json.loads(args.input.read_text(encoding="utf-8")))
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        parser.error(str(exc))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(result['rows'])} hypotheses")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
