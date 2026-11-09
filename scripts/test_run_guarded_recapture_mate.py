import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_guarded_recapture_mate import select_recaptures, validate_proof


class GuardedRecaptureMateTests(unittest.TestCase):
    def fixtures(self):
        cases = {"cases": [
            {"id": "pinned-guarded-focus", "sfen": "focus",
             "stages": [{"candidates": ["5h4h"]}]},
            {"id": "pinned-guarded-contrast", "sfen": "contrast", "stages": []}]}
        captures = [{"reply": reply, "reply_hash": board_hash,
                     "piece_present": False, "in_check": promoted,
                     "legal_recapture_count": 1,
                     "recapture_material": [{"move": "4h5g"}]}
                    for reply, board_hash, promoted in
                    [("5c5g", "a" * 16, False), ("5c5g+", "b" * 16, True)]]
        ledger = {"schema": "sekirei.strategy-reply-mobility-report.v4",
                  "status": "complete", "diagnostic_only": True,
                  "strength_claim": False, "completed_rows": 2,
                  "inputs": {"cases_sha256": "cases", "spec_sha256": "spec",
                             "material_ledger": True},
                  "rows": [{"case_id": "pinned-guarded-focus", "candidate": "5h4h",
                            "origin": "5g", "status": "complete", "reply_count": 2,
                            "reply_rows": captures},
                           {"case_id": "pinned-guarded-contrast"}]}
        return cases, ledger

    def test_selects_only_both_bound_capture_positions(self):
        cases, ledger = self.fixtures()
        selected = select_recaptures(ledger, cases, "cases", "spec")
        self.assertEqual([(row["reply"], row["reply_hash"]) for row in selected],
                         [("5c5g", "a" * 16), ("5c5g+", "b" * 16)])
        tampered = copy.deepcopy(ledger)
        tampered["rows"][0]["reply_rows"][1]["reply_hash"] = "bad"
        with self.assertRaisesRegex(ValueError, "verified legal king recapture"):
            select_recaptures(tampered, cases, "cases", "spec")
        with self.assertRaisesRegex(ValueError, "completed guarded"):
            select_recaptures(ledger, cases, "changed", "spec")

    def test_no_mate_requires_same_hash_and_no_abort(self):
        selection = {"case_id": "focus", "candidate": "5h4h", "reply": "5c5g",
                     "reply_hash": "a" * 16, "recapture": "4h5g"}
        fields = {"move": "4h5g", "initial_hash": "a" * 16,
                  "child_hash": "b" * 16, "max_ply": "7", "node_limit": "10000",
                  "outcome": "no_mate", "aborted": "false", "abort_reason": "none",
                  "invalid_reason": "none", "mate_ply": "0", "solutions": "",
                  "nodes": "71", "distance_rule": "complete"}
        self.assertEqual(validate_proof(fields, selection, 7, 10000)["outcome"], "no_mate")
        for key, value in [("initial_hash", "c" * 16), ("aborted", "true"),
                           ("nodes", "10001")]:
            changed = {**fields, key: value}
            with self.assertRaisesRegex(ValueError, "boundary"):
                validate_proof(changed, selection, 7, 10000)


if __name__ == "__main__":
    unittest.main()
