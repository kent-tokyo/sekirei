import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_candidate_teacher_probe import load_corpus, render_usi_commands  # noqa: E402


class ProbeTests(unittest.TestCase):
    def test_load_corpus_is_bounded_and_has_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory) / "corpus.jsonl"
            corpus.write_text(
                "\n".join(
                    json.dumps({"sample_id": f"sample-{index}", "sfen": f"sfen-{index}"})
                    for index in range(5)
                ),
                encoding="utf-8",
            )
            rows = load_corpus(corpus, 3)
        self.assertEqual(len(rows), 3)
        self.assertTrue(all(row["sample_id"] for row in rows))

    def test_render_usi_commands_preserves_one_command_per_line(self):
        rendered = render_usi_commands(["usi", "isready", "go depth 2"])
        self.assertEqual(rendered.count("printf"), 3)
        self.assertEqual(rendered.count(";"), 2)


if __name__ == "__main__":
    unittest.main()
