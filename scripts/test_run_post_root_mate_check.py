import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_post_root_mate_check as CHECK
from run_post_root_mate_check import parse_history, selected_ablation
from run_tactical_line_diagnostic import sha256


class PostRootMateCheckTests(unittest.TestCase):
    def test_replayed_hash_must_equal_traced_hash(self):
        line = "sfen=4k4/9/5P3/9/9/9/9/9/K8 w R 2\thash=" + "a" * 16 + "\n"
        self.assertIn("5P3", parse_history(line, "a" * 16))
        with self.assertRaises(ValueError):
            parse_history(line, "b" * 16)
        with self.assertRaises(ValueError):
            parse_history(line.strip() + "\thash=" + "a" * 16 + "\n", "a" * 16)

    def test_requires_complete_equal_depth_ablation(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source.json"
            row = lambda mode: {"mode": mode, "result": {"target_hash": "a" * 16,
                                "bestmove": "P*4c", "pv_usi": "P*4c,5a5b,R*4b",
                                "pv_legal": True, "search_board_restored": True,
                                "aborted": False}}
            report = {"schema": "sekirei.v2-quiet-trace-ablation.v1",
                      "status": "complete", "diagnostic_only": True,
                      "strength_claim": False,
                      "inputs": {"case_id": "focus", "cases_sha256": "b" * 64,
                                 "max_depth": 10, "root_move": "P*4c", "reply": "5a5b"},
                      "rows": [row("baseline"), row("target_exempt")]}
            path.write_text(json.dumps(report))
            self.assertEqual(selected_ablation(path, "b" * 64, "focus")["reply"], "5a5b")
            report["rows"][1]["result"]["target_hash"] = "c" * 16
            path.write_text(json.dumps(report))
            with self.assertRaises(ValueError):
                selected_ablation(path, "b" * 64, "focus")

    def test_timeout_retains_first_proof(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            sfen = "4k4/9/9/9/9/9/9/9/K8 b RP 1"
            cases = base / "cases.json"
            cases.write_text(json.dumps({"schema": "sekirei.tactical-line-cases.v1",
                "diagnostic_only": True, "cases": [
                    {"id": case_id, "sfen": sfen, "stages": [{"id": "root", "moves": [],
                     "expected_sfen": sfen, "candidates": ["P*4c"]}]}
                    for case_id in ("a", "b")]}))
            reports = []
            for case_id in ("a", "b"):
                path = base / f"{case_id}.json"
                path.write_text(json.dumps({
                    "schema": "sekirei.v2-quiet-trace-ablation.v1", "status": "complete",
                    "diagnostic_only": True, "strength_claim": False,
                    "inputs": {"case_id": case_id, "cases_sha256": sha256(cases),
                               "max_depth": 10, "root_move": "P*4c", "reply": "5a5b"},
                    "rows": [{"mode": mode, "result": {"target_hash": "a" * 16,
                              "bestmove": "P*4c", "pv_usi": "P*4c,5a5b",
                              "pv_legal": True, "search_board_restored": True,
                              "aborted": False}}
                             for mode in ("baseline", "target_exempt")]}))
                reports.append(path)
            history_bin, proof_bin = base / "history", base / "proof"
            history_bin.touch()
            proof_bin.touch()
            output = base / "out.json"
            argv = ["check", "--cases", str(cases), "--case-id", "a",
                    "--ablation-report", str(reports[0]), "--case-id", "b",
                    "--ablation-report", str(reports[1]), "--history-bin", str(history_bin),
                    "--proof-bin", str(proof_bin), "--output", str(output)]
            history_line = "sfen=4k4/9/5P3/9/9/9/9/9/K8 w R 2\thash=" + "a" * 16 + "\n"
            proof_line = ("move=5a5b\tinitial_hash=" + "a" * 16 +
                          "\tchild_hash=" + "b" * 16 +
                          "\tmax_ply=7\tnode_limit=10000\toutcome=no_mate\taborted=false"
                          "\tabort_reason=none\tinvalid_reason=none\tmate_ply=0"
                          "\tsolutions=\tnodes=12\tdistance_rule=all_legal_defender_replies\n")
            results = [SimpleNamespace(stdout=history_line),
                       SimpleNamespace(stdout=proof_line),
                       SimpleNamespace(stdout=history_line), TimeoutError("deadline")]
            with patch.object(sys, "argv", argv), patch.object(CHECK, "run_process",
                                                       side_effect=results):
                self.assertEqual(CHECK.main(), 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["completed_cases"], 1)
            self.assertEqual(report["rows"][0]["case_id"], "a")
            self.assertEqual(report["failure"]["case_id"], "b")


if __name__ == "__main__":
    unittest.main()
