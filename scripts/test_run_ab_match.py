#!/usr/bin/env python3
"""Unit tests for the SPRT helpers in run_ab_match.py (stdlib unittest only).

Run: python3 scripts/test_run_ab_match.py
"""

import math
import io
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_ab_match import main, sprt_bounds, sprt_llr  # noqa: E402


class _FakeMatchProcess:
    """Minimal stdlib-only `Popen` replacement for an early-stop regression."""

    def __init__(self, lines: list[str]):
        self._lines = iter(lines)
        self.stdout = self
        self.terminated = False
        self.lines_read = 0

    def __iter__(self):
        return self

    def __next__(self):
        line = next(self._lines)
        self.lines_read += 1
        return line

    def terminate(self):
        self.terminated = True

    def wait(self):
        return 143 if self.terminated else 0


class SprtTest(unittest.TestCase):
    def test_bounds_match_wald_for_five_percent_errors(self):
        lower, upper = sprt_bounds()
        self.assertAlmostEqual(lower, math.log(0.05 / 0.95))
        self.assertAlmostEqual(upper, math.log(0.95 / 0.05))

    def test_llr_sign_follows_the_score(self):
        self.assertEqual(sprt_llr(0, 0, 0, 0, 10), 0.0)
        self.assertGreater(sprt_llr(60, 40, 20, 0, 10), 0)
        self.assertLess(sprt_llr(40, 60, 20, 0, 10), 0)

    def test_strong_results_cross_the_bounds(self):
        lower, upper = sprt_bounds()
        # About +70 Elo over 600 games clearly accepts H1 for [0, 10].
        self.assertGreater(sprt_llr(300, 180, 120, 0, 10), upper)
        # About -70 Elo accepts H0.
        self.assertLess(sprt_llr(180, 300, 120, 0, 10), lower)
        # An even result stays between the bounds after 200 games.
        self.assertTrue(lower < sprt_llr(80, 80, 40, 0, 10) < upper)

    def test_llr_scales_with_games_at_a_fixed_score(self):
        small = sprt_llr(60, 40, 20, 0, 10)
        large = sprt_llr(600, 400, 200, 0, 10)
        self.assertAlmostEqual(large, 10 * small)

    def test_main_terminates_match_when_sprt_accepts_h1(self):
        # The process emits a result stream which would accept H1 well before
        # its configured 600-game upper bound.  No engine binary is needed.
        lines = (
            ["→ Engine1 Win\n"] * 300
            + ["→ Engine2 Win\n"] * 180
            + ["→ Draw\n"] * 120
        )
        process = _FakeMatchProcess(lines)
        with tempfile.TemporaryDirectory() as directory:
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--evalfile", "weights.nnue",
                "--games", "600",
                "--sprt", "0,10",
                "--out-dir", directory,
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch("sys.stdout", new_callable=io.StringIO) as stdout:
                self.assertEqual(main(), 0)

        self.assertTrue(process.terminated)
        self.assertLess(process.lines_read, len(lines))
        self.assertIn("H1 accepted", stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
