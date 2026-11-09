import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_guarded_check_evasions import parse_legal_moves, select_checks


class GuardedCheckEvasionsTests(unittest.TestCase):
    def fixtures(self):
        cases = {"cases": [
            {"id": "pinned-guarded-focus", "sfen": "focus",
             "stages": [{"candidates": ["5h4h"]}]},
            {"id": "pinned-guarded-contrast", "sfen": "contrast",
             "stages": [{"candidates": ["5h4h"]}]}]}
        focus = ["5c4c", "5c5g+"]
        contrast = ["3c4c", "3c3h", "3c3g+", "3c3h+", "3c3i+"]
        rows = []
        for case_id, moves in (("pinned-guarded-focus", focus),
                               ("pinned-guarded-contrast", contrast)):
            rows.append({"case_id": case_id, "status": "complete", "candidate": "5h4h",
                         "origin": "5g", "reply_count": len(moves),
                         "checking_reply_count": len(moves),
                         "reply_rows": [{"reply": move, "reply_hash": "a" * 16,
                                         "in_check": True, "piece_present": True,
                                         "material_after_reply_cp": -360}
                                        for move in moves]})
        ledger = {"schema": "sekirei.strategy-reply-mobility-report.v4",
                  "status": "complete", "diagnostic_only": True, "strength_claim": False,
                  "planned_rows": 2, "completed_rows": 2,
                  "inputs": {"cases_sha256": "cases", "spec_sha256": "spec",
                             "material_ledger": True}, "rows": rows}
        return cases, ledger

    def test_selects_seven_hash_bound_checks(self):
        cases, ledger = self.fixtures()
        selected = select_checks(ledger, cases, "cases", "spec")
        self.assertEqual(len(selected), 7)
        self.assertEqual(selected[0]["reply"], "5c4c")
        tampered = copy.deepcopy(ledger)
        tampered["rows"][1]["reply_rows"][0]["reply_hash"] = "wrong"
        with self.assertRaisesRegex(ValueError, "bound legal position"):
            select_checks(tampered, cases, "cases", "spec")
        tampered = copy.deepcopy(ledger)
        tampered["rows"][0]["checking_reply_count"] = 1
        with self.assertRaisesRegex(ValueError, "aggregate"):
            select_checks(tampered, cases, "cases", "spec")

    def test_legal_move_output_rejects_duplicates_and_malformed_rows(self):
        self.assertEqual(parse_legal_moves("4h3h 5g4f\n"), ["4h3h", "5g4f"])
        self.assertEqual(parse_legal_moves("\n"), [])
        for output in ("4h3h 4h3h\n", "bad\n", "4h3h\nextra\n"):
            with self.assertRaises(ValueError):
                parse_legal_moves(output)


if __name__ == "__main__":
    unittest.main()
