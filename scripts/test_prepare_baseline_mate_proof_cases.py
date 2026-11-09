import json
import tempfile
import unittest
from pathlib import Path

from prepare_baseline_mate_proof_cases import make_cases
from run_tactical_line_diagnostic import sha256


class BaselineMateProofCasesTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.cases = root / "cases.json"
        self.report = root / "report.json"
        self.sfen = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
        self.cases.write_text(json.dumps({
            "schema": "sekirei.tactical-line-cases.v1",
            "cases": [{"id": "one", "sfen": self.sfen,
                       "stages": [{"id": "first", "moves": [],
                                   "expected_sfen": self.sfen, "candidates": []}]}],
        }))
        self.data = {
            "schema": "sekirei.tactical-line-diagnostic.v1", "status": "complete",
            "inputs": {"cases_sha256": sha256(self.cases), "evaluator": "material"},
            "contract": {"threads": 1, "spec_top_n": 0},
            "rows": [{"case_id": "one", "stage_id": "first", "moves": [],
                      "position_sfen": self.sfen, "nodes_per_invocation": 1000,
                      "unrestricted": {"bestmove": "7g7f", "aborted": False,
                                       "completed_iteration_valid": True,
                                       "completed_bound": "exact", "pv_legal": True,
                                       "pv_usi": ["7g7f", "3c3d"]}}],
        }

    def write_report(self):
        self.report.write_text(json.dumps(self.data))

    def test_converts_exact_legal_baseline_and_binds_inputs(self):
        self.write_report()
        output = make_cases(self.cases, self.report, 1000)
        self.assertEqual(output["cases"][0]["stages"][0]["candidates"], ["7g7f"])
        self.assertEqual(output["source_report_sha256"], sha256(self.report))
        self.assertFalse(output["strength_claim"])

    def test_rejects_partial_and_non_exact_search(self):
        self.data["status"] = "interrupted"
        self.write_report()
        with self.assertRaisesRegex(ValueError, "complete"):
            make_cases(self.cases, self.report, 1000)
        self.data["status"] = "complete"
        self.data["rows"][0]["unrestricted"]["completed_bound"] = "lower"
        self.write_report()
        with self.assertRaisesRegex(ValueError, "no qualified"):
            make_cases(self.cases, self.report, 1000)

    def test_node_limit_abort_with_completed_exact_iteration_is_usable(self):
        self.data["rows"][0]["unrestricted"]["aborted"] = True
        self.write_report()
        self.assertEqual(len(make_cases(self.cases, self.report, 1000)["cases"]), 1)

    def test_explicit_pv_mismatch_flag_excludes_baseline(self):
        self.data["rows"][0]["unrestricted"]["bestmove_matches_pv_root"] = False
        self.write_report()
        with self.assertRaisesRegex(ValueError, "no qualified"):
            make_cases(self.cases, self.report, 1000)

    def test_explicit_completed_move_recovers_aborted_mismatch_only(self):
        result = self.data["rows"][0]["unrestricted"]
        result.update({"bestmove": "2g2f", "aborted": True,
                       "bestmove_matches_pv_root": False,
                       "completed_bestmove": "7g7f", "pv_root": "7g7f"})
        self.write_report()
        document = make_cases(self.cases, self.report, 1000)
        self.assertEqual(document["cases"][0]["stages"][0]["candidates"], ["7g7f"])
        self.assertEqual(document["recovered_completed_moves"], [{
            "case_id": "one", "stage_id": "first", "returned_bestmove": "2g2f",
            "completed_bestmove": "7g7f"}])
        result["aborted"] = False
        self.write_report()
        with self.assertRaisesRegex(ValueError, "no qualified"):
            make_cases(self.cases, self.report, 1000)

    def test_excludes_pv_bestmove_mismatch_without_losing_other_cases(self):
        document = json.loads(self.cases.read_text())
        second = json.loads(json.dumps(document["cases"][0]))
        second["id"] = "two"
        document["cases"].append(second)
        self.cases.write_text(json.dumps(document))
        self.data["inputs"]["cases_sha256"] = sha256(self.cases)
        second_row = json.loads(json.dumps(self.data["rows"][0]))
        second_row["case_id"] = "two"
        self.data["rows"].append(second_row)
        self.data["rows"][0]["unrestricted"]["bestmove"] = "2g2f"
        self.write_report()
        output = make_cases(self.cases, self.report, 1000)
        self.assertEqual([case["id"] for case in output["cases"]],
                         ["two-first-baseline-proof"])
        self.assertEqual(output["excluded"][0]["case_id"], "one")

    def test_rejects_report_from_other_corpus_and_missing_budget(self):
        self.data["inputs"]["cases_sha256"] = "0" * 64
        self.write_report()
        with self.assertRaisesRegex(ValueError, "bind"):
            make_cases(self.cases, self.report, 1000)
        self.data["inputs"]["cases_sha256"] = sha256(self.cases)
        self.write_report()
        with self.assertRaisesRegex(ValueError, "lacks"):
            make_cases(self.cases, self.report, 2000)


if __name__ == "__main__":
    unittest.main()
