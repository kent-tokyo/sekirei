import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_lazy_selected_coherence import parse_search


class SelectedCoherenceTests(unittest.TestCase):
    def test_selected_move_must_match_final_usi_move(self):
        lines = [
            "info string lazy_smp selected w1 node_share_permille 500 move_agreement 1/2 "
            "discarded_root_nodes 10 divergent_discarded_nodes 10 stop_lag_ms 0 "
            "selected_bestmove 1c1d selected_pv_root 8a9c "
            "selected_move_matches_pv_root false "
            "worker_bestmoves 2g2f,1c1d worker_pv_roots 2g2f,8a9c "
            "w0:d8:n10:s2:t1:x1:abudget:p0:h0:r0,"
            "w1*:d9:n10:s5:t1:x1:abudget:p0:h0:r0",
            "info depth 9 score cp 5 nodes 20 time 1 pv 8a9c",
            "bestmove 1c1d",
        ]
        row = parse_search(lines)
        self.assertFalse(row["matches_completed_pv"])
        self.assertEqual(row["usi_scored_pv_root"], "8a9c")
        self.assertEqual(row["worker_move_matches_pv_root"], [True, False])
        self.assertEqual(row["worker_depths"], [8, 9])
        with self.assertRaisesRegex(ValueError, "per-worker"):
            parse_search([line for line in lines if not line.startswith("info string lazy_smp")]
                         + [lines[0].replace("worker_bestmoves 2g2f,1c1d worker_pv_roots 2g2f,8a9c ", "")])
        with self.assertRaisesRegex(ValueError, "scored USI PV differs"):
            parse_search([lines[0], lines[1].replace("pv 8a9c", "pv 1c1d"), lines[2]])
        lines[-1] = "bestmove 8a9c"
        with self.assertRaisesRegex(ValueError, "differs"):
            parse_search(lines)


if __name__ == "__main__":
    unittest.main()
