import subprocess
import unittest
from pathlib import Path

from preflight_speed_corpus import preflight
from validate_speed_corpus import DEFAULT_CORPUS


class SpeedCorpusPreflightTest(unittest.TestCase):
    def test_runs_every_checked_in_case(self):
        calls = []

        def fake_runner(argv, **kwargs):
            calls.append(argv)
            return subprocess.CompletedProcess(argv, 0, "sfen_preflight=passed\n", "")

        count = preflight(Path("/bin/echo"), DEFAULT_CORPUS, fake_runner)
        self.assertEqual(count, 32)
        self.assertEqual(len(calls), 32)
        self.assertTrue(all(call[1] == "--check-sfen" for call in calls))

    def test_reports_a_failed_case(self):
        def failed_runner(argv, **kwargs):
            return subprocess.CompletedProcess(argv, 1, "", "bad fixture")

        with self.assertRaises(RuntimeError):
            preflight(Path("/bin/echo"), DEFAULT_CORPUS, failed_runner)


if __name__ == "__main__":
    unittest.main()
