import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_candidate_teacher_probe import load_corpus, render_usi_commands  # noqa: E402


class ProbeTests(unittest.TestCase):
    def test_load_corpus_is_bounded_and_has_ids(self):
        rows = load_corpus(Path("data/runs/nnue_v1_gate2/corpus100.jsonl"), 3)
        self.assertEqual(len(rows), 3)
        self.assertTrue(all(row["sample_id"] for row in rows))

    def test_render_usi_commands_preserves_one_command_per_line(self):
        rendered = render_usi_commands(["usi", "isready", "go depth 2"])
        self.assertEqual(rendered.count("printf"), 3)
        self.assertEqual(rendered.count(";"), 2)


if __name__ == "__main__":
    unittest.main()
