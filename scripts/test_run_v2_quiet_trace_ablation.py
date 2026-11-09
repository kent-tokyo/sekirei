import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_v2_quiet_trace_ablation import parse_mode, select_case
from run_v2_move_trace_diagnostic import TRACE_KEYS


class QuietTraceAblationTests(unittest.TestCase):
    def test_select_case_requires_explicit_candidate(self):
        case = {"id": "focus", "sfen": "x", "stages": [{"moves": [],
                "expected_sfen": "x", "candidates": ["P*4c"]}]}
        document = {"diagnostic_only": True, "legal_moves_sha256": "a" * 64,
                    "cases": [case]}
        self.assertEqual(select_case(document, "focus", "P*4c"), case)
        with self.assertRaises(ValueError):
            select_case(document, "focus", "R*5b")
        document["legal_moves_sha256"] = "invalid"
        with self.assertRaises(ValueError):
            select_case(document, "focus", "P*4c")

    def test_parse_mode_requires_mode_and_legal_fixed_root(self):
        fields = {"completed_iteration_valid": "true", "completed_bound": "exact",
                  "pv_legal": "true", "pv_replay_preserves_input": "true",
                  "search_board_restored": "true",
                  "bestmove": "P*4c", "score_cp": "1", "depth": "4", "nodes": "100",
                  "pv_usi": "P*4c,5a5b", "trace_target_move": "5a5b",
                  "trace_target_hash": "a" * 16, "v2_quiet_trace_exemption": "true"}
        fields.update({key: "0" for key in TRACE_KEYS})
        self.assertEqual(parse_mode(fields, "5a5b", "P*4c", True)["bestmove"], "P*4c")
        with self.assertRaises(ValueError):
            parse_mode(fields, "5a5b", "P*4c", False)
        fields["pv_usi"] = "R*5b"
        with self.assertRaises(ValueError):
            parse_mode(fields, "5a5b", "P*4c", True)


if __name__ == "__main__":
    unittest.main()
