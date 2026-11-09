import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_spec_prior_root_comparison import parse_trace


def trace_line(best="6h6g", pv="6h6g", iterations="1:9g9f:0:69:exact:0:0,2:6h6g:1:100:exact:0:0"):
    fields = {"bestmove": best, "pv_root": pv,
              "bestmove_matches_pv_root": str(best == pv).lower(),
              "pv_legal": "true", "pv_replay_preserves_input": "true",
              "completed_iteration_valid": "true", "completed_bound": "exact",
              "history_initial_hash": "a" * 16, "history_final_hash": "a" * 16,
              "iteration_trace": iterations,
              "nodes": "10000"}
    return "\t".join(f"{key}={value}" for key, value in fields.items()) + "\n"


class TraceTests(unittest.TestCase):
    def test_prior_move_comes_from_penultimate_completed_iteration(self):
        result = parse_trace(trace_line())
        self.assertEqual(result["prior_move"], "9g9f")
        self.assertEqual(result["bestmove"], "6h6g")

    def test_inconsistent_or_single_iteration_is_rejected(self):
        with self.assertRaises(ValueError):
            parse_trace(trace_line(best="8g8f", pv="6h6g"))
        with self.assertRaises(ValueError):
            parse_trace(trace_line(iterations="1:6h6g:1:100:exact:0:0"))


if __name__ == "__main__":
    unittest.main()
