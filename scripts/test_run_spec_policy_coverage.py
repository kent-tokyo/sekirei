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
import run_spec_policy_coverage as coverage


def probe_line(target="7g7f", rank="0"):
    moves = "9g9f,8g8f"
    fields = {"target": target, "initial_hash": "a" * 16,
              "legal_count": "30", "top_n": "2", "top_moves": moves,
              "target_in_top_n": "true" if rank != "0" else "false",
              "target_rank": rank, "tt_mode": "empty",
              "tt_hint": "none",
              "search_started": "false", "board_restored": "true"}
    return "\t".join(f"{key}={value}" for key, value in fields.items()) + "\n"


class SpecPolicyCoverageTests(unittest.TestCase):
    def test_rank_is_checked_against_actual_top_moves(self):
        result = coverage.parse_probe(probe_line(), "7g7f", 2)
        self.assertFalse(result["target_in_top_n"])
        with self.assertRaisesRegex(ValueError, "inconsistent target rank"):
            coverage.parse_probe(probe_line(rank="1"), "7g7f", 2)
        hit = coverage.parse_probe(probe_line(target="9g9f", rank="1"), "9g9f", 2)
        self.assertEqual(hit["target_rank"], 1)
        seeded = probe_line(target="9g9f", rank="1").replace(
            "tt_mode=empty", "tt_mode=seeded_root_hint"
        ).replace("tt_hint=none", "tt_hint=9g9f")
        self.assertEqual(coverage.parse_probe(seeded, "9g9f", 2, "9g9f")["target_rank"], 1)
        with self.assertRaisesRegex(ValueError, "contract"):
            coverage.parse_probe(seeded, "9g9f", 2)

    def test_timeout_preserves_prefix_without_coverage_fraction(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cases, probe_bin, output = root / "cases.json", root / "probe", root / "out.json"
            probe_bin.touch()
            sfen = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
            cases.write_text(json.dumps({
                "schema": "sekirei.tactical-line-cases.v1", "diagnostic_only": True,
                "strength_claim": False, "source_cases_sha256": "a" * 64,
                "source_report_sha256": "b" * 64, "nodes_per_baseline_search": 10000,
                "cases": [{"id": name, "sfen": sfen,
                           "stages": [{"id": "baseline", "candidates": ["7g7f"]}]}
                          for name in ("first", "second")],
            }))
            args = ["runner", "--cases", str(cases), "--probe-bin", str(probe_bin),
                    "--top-n", "2", "--max-invocations", "2", "--output", str(output)]
            effects = [subprocess.CompletedProcess([], 0, probe_line(), ""),
                       subprocess.TimeoutExpired("probe", 1)]
            with patch.object(sys, "argv", args), patch.object(
                coverage, "run_process", side_effect=effects
            ), redirect_stdout(io.StringIO()):
                self.assertEqual(coverage.main(), 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["status"], "interrupted")
            self.assertEqual(report["completed_invocations"], 1)
            self.assertIsNone(report["coverage_fraction"])
            self.assertEqual(report["failure"]["case_id"], "second")
            self.assertFalse(report["strength_claim"])


if __name__ == "__main__":
    unittest.main()
