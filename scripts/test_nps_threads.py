#!/usr/bin/env python3
"""Unit tests for explicit evaluator and thread-scaling options."""

import argparse
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from nps_threads import Usi, engine_options  # noqa: E402


class ThreadOptionsTest(unittest.TestCase):
    @staticmethod
    def args(*, yaneuraou: bool, evalfile: str | None) -> argparse.Namespace:
        return argparse.Namespace(
            yaneuraou=yaneuraou,
            evalfile=evalfile,
            fv_scale=24,
            hash=64,
            option=[],
        )

    def test_material_only_sekirei_omits_evalfile(self):
        options = engine_options(self.args(yaneuraou=False, evalfile=None), 4)

        self.assertIn("Threads=4", options)
        self.assertFalse(any(option.startswith("EvalFile=") for option in options))

    def test_explicit_sekirei_evalfile_is_preserved(self):
        options = engine_options(self.args(yaneuraou=False, evalfile="nn.bin"), 2)

        self.assertIn(f"EvalFile={Path('nn.bin').resolve()}", options)

    def test_relative_engine_path_is_resolved_before_changing_directory(self):
        with patch("nps_threads.subprocess.Popen") as popen:
            Usi("target/release/sekirei", {})

        binary = Path("target/release/sekirei").resolve()
        self.assertEqual(popen.call_args.args[0], [str(binary)])
        self.assertEqual(popen.call_args.kwargs["cwd"], str(binary.parent))


if __name__ == "__main__":
    unittest.main()
