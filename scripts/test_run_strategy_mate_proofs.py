"""Resource and partial-evidence tests for bounded strategy mate proofs."""

from __future__ import annotations

import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "mate_proofs", ROOT / "scripts" / "run_strategy_mate_proofs.py"
)
assert SPEC and SPEC.loader
PROOFS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROOFS)


class StrategyMateProofTests(unittest.TestCase):
    def fixture(self, directory: str, candidates: list[str]) -> tuple[Path, Path, Path]:
        root = Path(directory)
        engine = root / "proof"
        engine.touch()
        cases = root / "cases.json"
        sfen = "4k4/9/4r4/9/9/9/9/4G4/4K4 b - 1"
        cases.write_text(json.dumps({
            "schema": "sekirei.tactical-line-cases.v1",
            "cases": [{"id": "focus", "sfen": sfen,
                       "stages": [{"id": "first", "candidates": candidates}]}]
        }), encoding="utf-8")
        return engine, cases, root / "report.json"

    def argv(self, engine: Path, cases: Path, output: Path, limit: int) -> list[str]:
        return ["proofs", "--proof-bin", str(engine), "--cases", str(cases),
                "--max-invocations", str(limit), "--output", str(output)]

    def test_cap_rejects_before_any_proof_process(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            engine, cases, output = self.fixture(directory, ["5h5g", "5i4i"])
            stderr = io.StringIO()
            with patch.object(sys, "argv", self.argv(engine, cases, output, 1)), patch.object(
                PROOFS, "run_process", side_effect=AssertionError("proof started")
            ), redirect_stderr(stderr), self.assertRaises(SystemExit) as raised:
                PROOFS.main()
            self.assertEqual(raised.exception.code, 2)
            self.assertIn("requires 2 invocations", stderr.getvalue())
            self.assertFalse(output.exists())

    def test_timeout_saves_completed_prefix_and_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            engine, cases, output = self.fixture(directory, ["5h5g"])
            with patch.object(sys, "argv", self.argv(engine, cases, output, 1)), patch.object(
                PROOFS, "run_process", side_effect=subprocess.TimeoutExpired("proof", 1)
            ), redirect_stdout(io.StringIO()):
                self.assertEqual(PROOFS.main(), 1)
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["status"], "interrupted")
            self.assertEqual(report["completed_invocations"], 0)
            self.assertEqual(report["planned_invocations"], 1)
            self.assertEqual(report["failure"]["reason"], "TimeoutExpired")


if __name__ == "__main__":
    unittest.main()
