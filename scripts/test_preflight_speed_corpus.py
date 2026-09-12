import subprocess
import unittest
from pathlib import Path

from preflight_speed_corpus import preflight
from validate_speed_corpus import DEFAULT_CORPUS


class SpeedCorpusPreflightTest(unittest.TestCase):
    def test_runs_every_checked_in_case(self):
        calls = []
        expected = {
            case["sfen"]: case["expected"]
            for case in __import__("json").loads(DEFAULT_CORPUS.read_text())["cases"]
        }

        def fake_runner(argv, **kwargs):
            calls.append(argv)
            values = expected[argv[2]]
            if argv[1] == "--check-sequence":
                return subprocess.CompletedProcess(argv, 0, "sequence_preflight=passed;plies=1\n", "")
            if argv[1] == "--check-generated-sequence":
                return subprocess.CompletedProcess(argv, 0, "generated_sequence_preflight=passed;plies=12\n", "")
            output = (
                "sfen_preflight=passed\n"
                f"sfen_details=legal_moves:{values['legal_moves']};perft2:{values['perft2']}\n"
                f"sfen_moves={','.join(values['legal_moves_usi'])}\n"
                f"sfen_perft2_divide={','.join(values['perft2_divide'])}\n"
            )
            return subprocess.CompletedProcess(argv, 0, output, "")

        count = preflight(Path("/bin/echo"), DEFAULT_CORPUS, fake_runner)
        self.assertEqual(count, 128)
        self.assertEqual(len(calls), 384)
        self.assertEqual(sum(call[1] == "--check-sfen" for call in calls), 128)
        self.assertEqual(sum(call[1] == "--check-sequence" for call in calls), 128)
        self.assertEqual(sum(call[1] == "--check-generated-sequence" for call in calls), 128)

    def test_reports_a_failed_case(self):
        def failed_runner(argv, **kwargs):
            return subprocess.CompletedProcess(argv, 1, "", "bad fixture")

        with self.assertRaises(RuntimeError):
            preflight(Path("/bin/echo"), DEFAULT_CORPUS, failed_runner)


if __name__ == "__main__":
    unittest.main()
