import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_spec_selection_observation import parse_observation


def line(**changes):
    fields = {"initial_hash": "a" * 16, "bestmove": "7g7f",
              "bestmove_legal": "true", "pv_usi": "7g7f",
              "pv_legal": "true", "pv_root_matches_bestmove": "true",
              "completed_depth": "1", "nodes": "100", "requested_nodes": "100",
              "heuristics_mode": "fresh",
              "configured_workers": "3", "selected_groups": "1", "total_groups": "1",
              "selected_candidates": "2", "admitted_tasks": "1", "cap_rejected": "1",
              "started_tasks": "1", "completed_tasks": "0", "aborted_tasks": "1",
              "peak_in_flight": "1", "background_nodes": "20",
              "permit_hold_ns_sum": "10", "permit_hold_ns_max": "10",
              "queue_wait_ns_sum": "3", "queue_wait_ns_max": "3",
              "started_hold_ns_sum": "7", "started_hold_ns_max": "7",
              "unstarted_hold_ns_sum": "0",
              "child_preflight_ns_sum": "2", "child_kernel_ns_sum": "4",
              "child_legal_check_ns_sum": "1",
              "child_state_setup_ns_sum": "1",
              "tt_attribution_available": "true",
              "tt_score_hits_from_speculation": "0",
              "tt_move_hits_from_speculation": "0",
              "tt_main_cutoffs_from_speculation": "0",
              "tt_nonroot_move_first_started": "0",
              "outstanding_after_join": "0",
              "selection_trace": "1:none:9g9f,8g8f:2:9g9f:1:0:0",
              "board_restored": "true", "diagnostic_only": "true"}
    fields.update(changes)
    return "\t".join(f"{key}={value}" for key, value in fields.items()) + "\n"


class ObservationTests(unittest.TestCase):
    def test_parse_selected_and_admitted_separately(self):
        result = parse_observation(line(), 100)
        self.assertEqual(result["groups"][0]["selected"], ["9g9f", "8g8f"])
        self.assertEqual(result["groups"][0]["admitted"], ["9g9f"])

    def test_reject_inconsistent_admission_and_board(self):
        with self.assertRaises(ValueError):
            parse_observation(line(cap_rejected="0"), 100)
        with self.assertRaises(ValueError):
            parse_observation(line(board_restored="false"), 100)
        with self.assertRaises(ValueError):
            parse_observation(line(queue_wait_ns_sum="4"), 100)
        with self.assertRaises(ValueError):
            parse_observation(line(child_kernel_ns_sum="10"), 100)
        with self.assertRaises(ValueError):
            parse_observation(line(child_legal_check_ns_sum="3"), 100)
        with self.assertRaises(ValueError):
            parse_observation(line(child_state_setup_ns_sum="3"), 100)
        with self.assertRaises(ValueError):
            parse_observation(line(pv_usi="9g9f"), 100)
        shared = line(heuristics_mode="shared")
        self.assertEqual(parse_observation(shared, 100, "shared")["heuristics_mode"], "shared")
        with self.assertRaises(ValueError):
            parse_observation(shared, 100)
        background = line(heuristics_mode="background_shared")
        self.assertEqual(parse_observation(background, 100, "background_shared")["heuristics_mode"],
                         "background_shared")
        isolated = line(heuristics_mode="background_isolated_tt")
        self.assertEqual(parse_observation(isolated, 100, "background_isolated_tt")["heuristics_mode"],
                         "background_isolated_tt")
        with self.assertRaises(ValueError):
            parse_observation(line(heuristics_mode="background_isolated_tt",
                                   tt_score_hits_from_speculation="1"), 100,
                              "background_isolated_tt")


if __name__ == "__main__":
    unittest.main()
