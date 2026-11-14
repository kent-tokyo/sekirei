import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from analyze_paired_openings import analyze, material, material_bucket, pair_outcome, sha, verify_kifu


EMPTY = "9/9/9/9/9/9/9/9/9"


class OpeningAnalysisTest(unittest.TestCase):
    def test_standard_position_and_perspective(self):
        board = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL"
        self.assertEqual(material(board + " b - 1"), 0)
        self.assertEqual(material(EMPTY + " b 2P3p 1"), -100)
        self.assertEqual(material(EMPTY + " w 2P3p 1"), -100)

    def test_promoted_pieces_and_hands_are_counted(self):
        self.assertEqual(material("4+P4/9/9/9/9/9/9/9/9 b - 1"), 600)
        self.assertEqual(material("4+r4/9/9/9/9/9/9/9/9 w R 2"), -260)

    def test_malformed_sfen_is_rejected(self):
        for value in ("8/9/9/9/9/9/9/9/9 b - 1", "4+K4/9/9/9/9/9/9/9/9 b - 1",
                      EMPTY + " b 0P 1", EMPTY + " b PP 1", EMPTY + " x - 1", EMPTY + " b - 0"):
            with self.subTest(value=value), self.assertRaises(ValueError): material(value)

    def test_pair_categories_preserve_single_draws(self):
        self.assertEqual(pair_outcome("candidate_win", "baseline_win"), "split")
        self.assertEqual(pair_outcome("draw", "draw"), "double_draw")
        self.assertEqual(pair_outcome("candidate_win", "draw"), "candidate_pair_win")
        self.assertEqual(pair_outcome("draw", "baseline_win"), "candidate_pair_loss")
        with self.assertRaises(ValueError): pair_outcome("error", "draw")

    def test_bucket_boundaries(self):
        self.assertEqual([material_bucket(n) for n in (0, -1, 300, -301, 1000, -1001)],
                         ["0", "1..300", "1..300", "301..1000", "301..1000", ">1000"])

    def test_kifu_root_and_colour_cannot_be_swapped(self):
        text = "# Engine1: engine (Black)\n# Engine2: engine (White)\n# Result: Engine1 Win\nposition startpos moves 7g7f 3c3d\n"
        verify_kifu(text, "startpos moves 7g7f", "candidate_win", "Black")
        for opening, result, colour in (("startpos moves 2g2f", "candidate_win", "Black"),
                                       ("startpos moves 7g7f", "draw", "Black"),
                                       ("startpos moves 7g7f", "candidate_win", "White")):
            with self.assertRaises(ValueError): verify_kifu(text, opening, result, colour)

    def test_draw_reason_is_preserved_without_becoming_a_win(self):
        text = "# Engine1: engine (Black)\n# Engine2: engine (White)\n# Result: Draw (千日手)\nposition startpos moves 7g7f\n"
        verify_kifu(text, "startpos moves 7g7f", "draw", "Black")
        with self.assertRaises(ValueError): verify_kifu(text, "startpos moves 7g7f", "candidate_win", "Black")
        with self.assertRaises(ValueError): verify_kifu(text + "# Result: Draw\n", "startpos moves 7g7f", "draw", "Black")

    def test_unfinished_trial_is_rejected_before_outcome_analysis(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "completed, finalized"):
                analyze(Path(directory))

    def test_frozen_completed_fixture_and_tamper_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def save(name, value):
                path = root / name
                path.write_text(json.dumps(value))
                return path
            manifest = save("roots.json", {"positions": [{"sfen": EMPTY + " b P 1", "position": "startpos moves 7g7f",
                "ply": 24, "identity_sha256": "fixture"}]})
            plan = save("plan.json", {"games_cap": 2, "pins": {"root_manifest": {"path": str(manifest), "sha256": sha(manifest)}}})
            events = root / "events.jsonl"
            events.write_text('\n'.join(json.dumps({"id": "pos0_pair0", "result": outcome})
                              for outcome in ("candidate_win", "baseline_win")) + '\n')
            files = []
            for index, colour, result in ((1, "Black", "Engine1 Win"), (2, "White", "Engine2 Win")):
                other = "White" if colour == "Black" else "Black"
                path = root / f"game{index:04}.txt"
                path.write_text(f"# Engine1: engine ({colour})\n# Engine2: engine ({other})\n# Result: {result}\nposition startpos moves 7g7f 3c3d\n")
                files.append({"name": path.name, "sha256": sha(path)})
            receipt = save("match-result.json", {"status": "completed", "evidence": {"complete": True,
                "outputs": {"raw_match_jsonl": {"path": str(events)},
                            "kifu_manifest": {"directory": str(root), "file_count": 2, "files": files}}}})
            save("decision.json", {"status": "UNCHANGED_OLD_VERDICT", "candidate": "fixture", "games": 2,
                "plan_sha256": sha(plan), "root_manifest_sha256": sha(manifest), "events_sha256": sha(events),
                "match_result_sha256": sha(receipt), "pair_observations": {"wins": 0, "losses": 0, "draws": 1}})
            result = analyze(root)
            self.assertFalse(result["old_verdict_changed"])
            self.assertEqual(result["summary"]["outcomes"]["split"], 1)
            self.assertEqual(result["summary"]["material_sign_predicts_split_winner"], {"eligible_pairs": 1, "matching_pairs": 1})
            (root / "game0001.txt").write_text("changed")
            with self.assertRaisesRegex(ValueError, "kifu hash"): analyze(root)


if __name__ == "__main__": unittest.main()
