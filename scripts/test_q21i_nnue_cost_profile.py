#!/usr/bin/env python3
"""Unit tests for the Q21i cost-profile contract."""

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "q21i", ROOT / "scripts/q21i_nnue_cost_profile.py"
)
Q21I = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(Q21I)


class Q21iCostProfileTests(unittest.TestCase):
    def test_selection_covers_nine_strata_and_both_classes(self):
        entries = []
        rank = 0
        for phase in Q21I.PHASES:
            for band in Q21I.MATERIAL_BANDS:
                for selection_class in ("normal", "tactical"):
                    entries.append(
                        {
                            "id": f"p{rank}",
                            "selection_class": selection_class,
                            "selection_rank": f"{rank:04d}",
                            "attributes": {"phase": phase, "material_band": band},
                        }
                    )
                    rank += 1
        selected = Q21I.select_positions(entries)
        self.assertEqual(len(selected), 9)
        self.assertEqual(
            {(e["attributes"]["phase"], e["attributes"]["material_band"]) for e in selected},
            {(phase, band) for phase in Q21I.PHASES for band in Q21I.MATERIAL_BANDS},
        )
        self.assertEqual({e["selection_class"] for e in selected}, {"normal", "tactical"})

    def test_orders_are_balanced(self):
        for arm in Q21I.ARMS:
            self.assertEqual(sorted(order.index(arm) for order in Q21I.SEARCH_ORDERS), [0, 1, 2])
        for mode in Q21I.COMPONENT_MODES:
            self.assertEqual(sorted(order.index(mode) for order in Q21I.COMPONENT_ORDERS), [0, 1, 2])

    def test_aggregation_uses_per_position_medians(self):
        rows = []
        for budget in ("fixed_nodes", "fixed_time"):
            for position_id, base in (("a", 10), ("b", 100)):
                for repetition, factor in enumerate((1, 9, 2)):
                    for arm_index, arm in enumerate(Q21I.ARMS, start=1):
                        value = base * factor * arm_index
                        rows.append(
                            {
                                "budget": budget,
                                "position_id": position_id,
                                "repetition": repetition,
                                "arm": arm,
                                "result": {
                                    "depth": value,
                                    "nodes": value,
                                    "elapsed_ms": value,
                                    "static_evaluations": value,
                                    "eval_cache_probes": value,
                                    "eval_cache_hits": value // 2,
                                    "max_rss_bytes": value * 1000,
                                    "bestmove": "7g7f",
                                    "score_cp": 0,
                                },
                            }
                        )
        components = []
        for repetition in range(3):
            for mode_index, mode in enumerate(Q21I.COMPONENT_MODES, start=1):
                components.append(
                    {
                        "repetition": repetition,
                        "mode": mode,
                        "result": {
                            "cases": [
                                {"name": fixture, "median_ns": mode_index * 10 + repetition}
                                for fixture in ("quiet", "capture", "drop")
                            ]
                        },
                    }
                )
        summary = Q21I.aggregate(rows, components)
        # Per-position medians are 20 and 200; their median is 110.
        self.assertEqual(summary["corpus_medians"]["fixed_nodes"]["material"]["nodes"], 110)
        self.assertEqual(
            summary["corpus_medians"]["fixed_nodes"]["material"]["eval_cache_probes"],
            110,
        )
        self.assertAlmostEqual(
            summary["corpus_medians"]["fixed_nodes"]["material"]["eval_cache_hit_rate"],
            0.5,
        )
        self.assertEqual(summary["components"]["quiet"]["material"], 11)
        self.assertEqual(summary["zero_scale_fixed_node_identity"]["matching"], 6)
        self.assertIn("median_mixed_remainder_ms", summary["forward_attribution"])
        self.assertEqual(
            summary["fixed_time_variability"]["material"]["positions_with_bestmove_variation"],
            0,
        )

    def test_parser_accepts_and_types_cache_counters(self):
        result = Q21I.parse_profile(
            "\t".join(
                (
                    "bestmove=7g7f",
                    "depth=4",
                    "score_cp=12",
                    "nodes=100",
                    "elapsed_ms=1",
                    "bound=exact",
                    "completed_bound=exact",
                    "completed_iteration_valid=true",
                    "aborted=false",
                    "abort_reason=none",
                    "static_evaluations=40",
                    "eval_cache_probes=40",
                    "eval_cache_hits=10",
                    "pv_legal=true",
                    "pv_replay_preserves_input=true",
                    "history_matches_expected=true",
                )
            )
        )
        self.assertEqual(result["eval_cache_probes"], 40)
        self.assertEqual(result["eval_cache_hits"], 10)


if __name__ == "__main__":
    unittest.main()
