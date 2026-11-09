import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_guarded_narrow_escape_mate import select_narrow_check, validate_proof


class NarrowEscapeMateTests(unittest.TestCase):
    def test_requires_both_reported_escapes_and_ledger_hash(self):
        target = {"case_id": "pinned-guarded-contrast", "reply": "3c3h+",
                  "reply_hash": "a" * 16, "candidate": "5h4h", "checkmate_now": False,
                  "legal_evasions": ["4h3h", "4h5i"],
                  "king_evasions": ["4h3h", "4h5i"]}
        report = {"schema": "sekirei.guarded-check-evasions.v1", "status": "complete",
                  "diagnostic_only": True, "strength_claim": False,
                  "planned_rows": 7, "completed_rows": 7,
                  "inputs": {"cases_sha256": "cases", "ledger_sha256": "ledger"},
                  "rows": [target] + [{"case_id": "other"} for _ in range(6)]}
        ledger = {"schema": "sekirei.strategy-reply-mobility-report.v4",
                  "status": "complete", "rows": [
                      {"case_id": "pinned-guarded-contrast", "reply_rows": [
                          {"reply": "3c3h+", "reply_hash": "a" * 16,
                           "in_check": True}]}]}
        self.assertEqual(select_narrow_check(report, ledger, "cases", "ledger"), target)
        with self.assertRaisesRegex(ValueError, "completed guarded source"):
            select_narrow_check(report, ledger, "cases", "changed")
        target["legal_evasions"] = ["4h3h"]
        with self.assertRaisesRegex(ValueError, "both king escapes"):
            select_narrow_check(report, ledger, "cases", "ledger")

    def test_no_mate_cannot_be_aborted_or_misbound(self):
        fields = {"move": "4h3h", "initial_hash": "a" * 16,
                  "child_hash": "b" * 16, "max_ply": "7", "node_limit": "10000",
                  "outcome": "no_mate", "aborted": "false", "abort_reason": "none",
                  "invalid_reason": "none", "mate_ply": "0", "solutions": "",
                  "nodes": "4", "distance_rule": "all_legal_defender_replies"}
        self.assertEqual(validate_proof(fields, "a" * 16, "4h3h", 7, 10000)["nodes"], 4)
        for changed in ({"initial_hash": "c" * 16}, {"aborted": "true"},
                        {"nodes": "10001"}):
            with self.assertRaisesRegex(ValueError, "boundary"):
                validate_proof({**fields, **changed}, "a" * 16, "4h3h", 7, 10000)


if __name__ == "__main__":
    unittest.main()
