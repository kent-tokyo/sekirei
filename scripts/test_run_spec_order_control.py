import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_spec_order_control import MODES, ORDER, summarize, validate_confirmation


class OrderControlTests(unittest.TestCase):
    def test_balanced_order_and_summary(self):
        self.assertEqual(ORDER, MODES + tuple(reversed(MODES)))
        rows = [{"case_id": "a", "initial_hash": "a" * 16,
                 "heuristics_mode": mode, "bestmove": move,
                 "completed_depth": depth, "started_tasks": 1,
                 "tt_score_hits_from_speculation": 0}
                for mode in MODES for move, depth in (("7g7f", 2), ("2g2f", 3))]
        result = summarize(rows, ["a"])
        self.assertEqual([item["distinct_bestmoves"] for item in result], [2, 2, 2])
        with self.assertRaises(ValueError):
            summarize(rows[:-1], ["a"])
        rows[0]["initial_hash"] = "b" * 16
        with self.assertRaises(ValueError):
            summarize(rows, ["a"])

    def test_confirmation_provenance_and_direct_root(self):
        digest = "a" * 64
        case = lambda name: {"id": name, "sfen": "x", "source": {"manifest_sha256": digest},
                             "stages": [{"moves": [], "expected_sfen": "x"}]}
        document = {"role": "confirmation", "diagnostic_only": True,
                    "source_manifest_sha256": digest, "cases": [case("a"), case("b")]}
        self.assertEqual(set(validate_confirmation(document, ["a", "b"])), {"a", "b"})
        document["cases"][1]["stages"][0]["moves"] = ["7g7f"]
        with self.assertRaises(ValueError):
            validate_confirmation(document, ["a", "b"])


if __name__ == "__main__":
    unittest.main()
