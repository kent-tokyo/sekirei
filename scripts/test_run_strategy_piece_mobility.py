import io
import json
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_strategy_piece_mobility as mobility
from build_handcrafted_strategy_cases import position


class PieceMobilityTests(unittest.TestCase):
    def test_measure_uses_same_mover_and_rejects_unlisted_move(self):
        initial = position({"5i": "K", "5a": "k", "9i": "B", "8h": "P"}, "-")
        child = position({"5i": "K", "5a": "k", "9i": "B", "8g": "P"}, "-")
        fields = f"sfen={child.replace(' b ', ' w ')}\thash={'1' * 16}\n"
        case = {"id": "focus", "sfen": initial}
        before = ["8h8g", "5i4i"]
        after = ["9i8h", "9i7g", "5i4i"]
        with patch.object(mobility, "run_process", return_value=subprocess.CompletedProcess(
            [], 0, fields, ""
        )), patch.object(mobility, "legal_moves_for_two", return_value=(before, after)):
            result = mobility.measure(case, "8h8g", "9i", Path("legal"), Path("history"),
                                      time.monotonic() + 10)
            self.assertEqual((result["before_count"], result["after_count"]), (0, 2))
            self.assertEqual(result["hypothetical_same_side_sfen"].split()[1], "b")
            with self.assertRaisesRegex(ValueError, "not legal"):
                mobility.measure(case, "8h8f", "9i", Path("legal"), Path("history"),
                                 time.monotonic() + 10)

    def test_failure_keeps_completed_row_and_candidate_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cases, spec = root / "cases.json", root / "spec.json"
            legal, history, output = root / "legal", root / "history", root / "out.json"
            legal.touch()
            history.touch()
            sfen = position({"5i": "K", "5a": "k", "9i": "B", "8h": "P"}, "-")
            cases.write_text(json.dumps({"schema": "sekirei.tactical-line-cases.v1",
                "cases": [{"id": name, "sfen": sfen,
                           "stages": [{"id": "first", "moves": [],
                                       "candidates": ["8h8g"]}]}
                          for name in ("first", "second")]}))
            spec.write_text(json.dumps({"schema": mobility.SCHEMA,
                "selections": [{"case_id": name, "candidate": "8h8g", "origin": "9i"}
                               for name in ("first", "second")]}))
            args = ["runner", "--cases", str(cases), "--spec", str(spec),
                    "--legal-moves", str(legal), "--history-sfen", str(history),
                    "--max-invocations", "4", "--output", str(output)]
            first = {"case_id": "first", "candidate": "8h8g", "before_count": 0,
                     "after_count": 2, "delta": 2}
            with patch.object(sys, "argv", args), patch.object(
                mobility, "measure", side_effect=[first, TimeoutError("deadline")]
            ), redirect_stdout(io.StringIO()):
                self.assertEqual(mobility.main(), 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["status"], "interrupted")
            self.assertEqual(report["rows"], [first])
            self.assertEqual(report["failure"]["case_id"], "second")
            self.assertFalse(report["strength_claim"])


if __name__ == "__main__":
    unittest.main()
