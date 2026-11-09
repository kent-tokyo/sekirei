import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_strategy_threat_probes as threat


def result_line(*, status="all_replies_allow_mate_in_one", replies="2", mates="2"):
    fields = {
        "move": "5c5g+", "initial_hash": "1" * 16, "child_hash": "2" * 16,
        "status": status, "max_replies": "64", "replies": replies,
        "replies_with_mate_in_one": mates,
        "first_escape": "none" if status != "has_escape_from_mate_in_one" else "5i4i",
        "gives_check": "true", "is_capture": "true", "board_restored": "true",
    }
    return "\t".join(f"{key}={value}" for key, value in fields.items()) + "\n"


class ThreatProbeRunnerTests(unittest.TestCase):
    def test_accepts_complete_proof_and_rejects_inconsistent_claim(self):
        result = threat.parse_probe(result_line(), "5c5g+", 64)
        self.assertEqual(result["status"], "all_replies_allow_mate_in_one")
        self.assertEqual(result["replies_with_mate_in_one"], 2)
        with self.assertRaisesRegex(ValueError, "requires every legal reply"):
            threat.parse_probe(result_line(mates="1"), "5c5g+", 64)
        with self.assertRaisesRegex(ValueError, "mismatched"):
            threat.parse_probe(result_line(), "5c5f", 64)
        malformed = result_line().replace("gives_check=true", "gives_check=maybe")
        with self.assertRaisesRegex(ValueError, "invalid boolean"):
            threat.parse_probe(malformed, "5c5g+", 64)

    def test_timeout_keeps_completed_prefix_and_failure_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cases, probe_bin, output = (root / "cases.json", root / "probe", root / "out.json")
            probe_bin.touch()
            sfen = "4k4/9/4r4/9/9/9/4G4/9/4K4 w - 2"
            cases.write_text(json.dumps({"schema": "sekirei.tactical-line-cases.v1",
                "cases": [{"id": "case", "sfen": sfen,
                           "stages": [{"id": "first", "moves": [], "expected_sfen": sfen,
                                       "candidates": ["5c5g+", "5c5d"]}]}]}))
            args = ["runner", "--cases", str(cases), "--probe-bin", str(probe_bin),
                    "--max-invocations", "2", "--max-total-seconds", "10",
                    "--output", str(output)]
            effects = [subprocess.CompletedProcess([], 0, result_line(), ""),
                       subprocess.TimeoutExpired("probe", 1)]
            with patch.object(sys, "argv", args), patch.object(
                threat, "run_process", side_effect=effects
            ), redirect_stdout(io.StringIO()):
                self.assertEqual(threat.main(), 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["status"], "interrupted")
            self.assertEqual(report["completed_invocations"], 1)
            self.assertEqual(report["rows"][0]["candidate"], "5c5g+")
            self.assertEqual(report["failure"]["candidate"], "5c5d")
            self.assertFalse(report["strength_claim"])


if __name__ == "__main__":
    unittest.main()
