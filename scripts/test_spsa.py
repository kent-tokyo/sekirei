"""Stdlib-only contract tests for the material-only SPSA runner."""

import contextlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("spsa", ROOT / "scripts" / "spsa.py")
SPSA = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(SPSA)


class ArgumentContractTest(unittest.TestCase):
    def setUp(self):
        self.common = [
            "--engine",
            "target/release/sekirei",
            "--params",
            "params.txt",
            "--games",
            "2",
            "--openings",
            "openings.sfen",
        ]

    def assert_rejected(self, argv):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            SPSA.parse_args(argv)

    def test_material_baseline_is_explicit_and_exclusive(self):
        self.assert_rejected(self.common)
        self.assert_rejected(self.common + ["--baseline", "external"])
        self.assert_rejected(
            self.common + ["--baseline", "material", "--eval-file", "network.bin"]
        )

    def test_colour_swapped_pair_count_is_required(self):
        odd = self.common.copy()
        odd[odd.index("2")] = "3"
        self.assert_rejected(odd + ["--baseline", "material"])

    def test_valid_material_contract_uses_workspace_match_binary(self):
        args = SPSA.parse_args(self.common + ["--baseline", "material"])
        self.assertEqual(args.baseline, "material")
        self.assertEqual(args.match, "target/release/sekirei-match")


class RunnerContractTest(unittest.TestCase):
    def make_args(self, directory: str) -> SimpleNamespace:
        opening = Path(directory) / "openings.sfen"
        opening.write_text("startpos\n", encoding="utf-8")
        return SimpleNamespace(
            games=2,
            state=str(Path(directory) / "state.json"),
            openings=str(opening),
            baseline="material",
            match="target/release/sekirei-match",
            engine="target/release/sekirei",
            byoyomi=10,
        )

    @staticmethod
    def parameters():
        return [
            {
                "name": "RFP_MARGIN",
                "start": 152.0,
                "c_end": 2.0,
                "r_end": 0.002,
                "min": 20,
                "max": 400,
            }
        ]

    def test_match_command_contains_only_material_contract_options(self):
        with tempfile.TemporaryDirectory() as directory:
            tuner = SPSA.Spsa(self.make_args(directory), self.parameters())
            job = {
                "plus": {"RFP_MARGIN": 154},
                "minus": {"RFP_MARGIN": 150},
                "opening": "startpos",
            }
            completed = mock.Mock(
                returncode=0,
                stdout="=== Results after 2 games ===\nEngine1 Win\n→ Draw\n",
                stderr="",
            )
            with mock.patch.object(
                SPSA.subprocess, "run", return_value=completed
            ) as run:
                self.assertEqual(tuner.play(job), (1, 0, 1))

        command = run.call_args.args[0]
        joined = " ".join(command)
        self.assertNotIn("EvalFile", joined)
        self.assertNotIn("NnueOutput", joined)
        self.assertNotIn("FV_SCALE", joined)
        for side in ("--engine-option1", "--engine-option2"):
            selected = [
                command[index + 1]
                for index, item in enumerate(command[:-1])
                if item == side
            ]
            self.assertIn("Threads=1", selected)
            self.assertIn("SpecTopN=0", selected)
            self.assertIn("UseBook=false", selected)
            self.assertTrue(
                any(item.startswith("T_RFP_MARGIN=") for item in selected)
            )

    def test_legacy_state_without_evaluator_contract_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            args = self.make_args(directory)
            old_state = Path(directory) / "old-external-state.json"
            old_state.write_text(
                '{"n_iter": 1, "theta": {"RFP_MARGIN": 152}, "done": 0}',
                encoding="utf-8",
            )
            args.state = str(old_state)
            with self.assertRaisesRegex(SystemExit, "evaluator contract"):
                SPSA.Spsa(args, self.parameters())


if __name__ == "__main__":
    unittest.main()
