#!/usr/bin/env python3

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "compare_teacher_evals", ROOT / "compare_teacher_evals.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)
compare = MODULE.compare
pearson = MODULE.pearson


def record(sample_id, score, *, nodes, time_ms, wall_time_ms, status="ok"):
    lines = [] if score is None else [{"score_cp": score, "nodes": nodes, "time_ms": time_ms}]
    return {
        "sample_id": sample_id,
        "status": status,
        "wall_time_ms": wall_time_ms,
        "lines": lines,
    }


class TeacherEvalComparisonTest(unittest.TestCase):
    def write_jsonl(self, path, records):
        path.write_text(
            "".join(json.dumps(item) + "\n" for item in records), encoding="utf-8"
        )

    def test_score_and_cost_contract(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            material_path = root / "material.jsonl"
            nnue_path = root / "nnue.jsonl"
            self.write_jsonl(
                material_path,
                [
                    record("a", -2, nodes=10, time_ms=2, wall_time_ms=3),
                    record("b", 0, nodes=20, time_ms=4, wall_time_ms=5),
                    record("c", 2, nodes=30, time_ms=6, wall_time_ms=7),
                    record("timeout", None, nodes=0, time_ms=0, wall_time_ms=11, status="timeout"),
                ],
            )
            self.write_jsonl(
                nnue_path,
                [
                    record("a", -1, nodes=20, time_ms=4, wall_time_ms=6),
                    record("b", 1, nodes=40, time_ms=8, wall_time_ms=10),
                    record("c", 3, nodes=60, time_ms=12, wall_time_ms=14),
                    record("timeout", 4, nodes=80, time_ms=16, wall_time_ms=18),
                ],
            )

            report = compare(material_path, nnue_path)
            self.assertEqual(report["coverage"]["shared_ok_records"], 3)
            self.assertEqual(report["scores"]["positions"], 3)
            self.assertAlmostEqual(report["scores"]["pearson_correlation"], 1.0)
            self.assertEqual(report["scores"]["sign_mismatch_count"], 1)
            self.assertEqual(report["scores"]["opposite_nonzero_sign_count"], 0)
            self.assertAlmostEqual(report["scores"]["material"]["variance_cp2"], 8 / 3)
            ratios = report["search_cost_on_shared_ok"]["fixed_nnue_over_material"]
            self.assertAlmostEqual(ratios["nodes_total_ratio"], 2.0)
            self.assertAlmostEqual(ratios["search_time_total_ratio"], 2.0)
            self.assertAlmostEqual(ratios["wall_time_total_ratio"], 2.0)

    def test_constant_input_has_undefined_correlation(self):
        self.assertIsNone(pearson([1.0, 1.0], [2.0, 3.0]))

    def test_duplicate_sample_id_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "duplicate.jsonl"
            duplicate = record("same", 1, nodes=1, time_ms=1, wall_time_ms=1)
            self.write_jsonl(path, [duplicate, duplicate])
            with self.assertRaisesRegex(ValueError, "duplicate sample_id"):
                compare(path, path)


if __name__ == "__main__":
    unittest.main()
