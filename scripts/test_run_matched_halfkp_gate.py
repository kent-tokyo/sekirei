#!/usr/bin/env python3
"""Pair integrity and no-silent-change tests for the matched gate driver."""
import json
from pathlib import Path
import tempfile
import unittest

from scripts import run_matched_halfkp_gate as G


class MatchedGateTests(unittest.TestCase):
    def plan(self):
        return {"pins": {name: {"path": f"/tmp/{name}", "sha256": "0" * 64}
                         for name in ("engine", "match", "hypothesis", "control")},
                "conditions": {"byoyomi_ms": 200, "max_moves": 512}}

    def make_pair(self, root: Path):
        pair = root / "pair-0000"
        (pair / "kifu").mkdir(parents=True)
        for number in (1, 2):
            (pair / "kifu" / f"game{number:04d}.txt").write_text("record\n")
        plan = self.plan()
        summary = {"status": "complete", "games": 2,
                   "artifact_files_expected": 2, "artifact_write_failures": [],
                   "invalid_games": [], "engine1_wins": 1, "engine2_wins": 1,
                   "draws": 0}
        for arm, weight in ((1, "hypothesis"), (2, "control")):
            path = plan["pins"][weight]["path"]
            summary[f"engine{arm}_command"] = plan["pins"]["engine"]["path"]
            summary[f"engine{arm}_options"] = {
                "EvalFile": path, "Threads": "1", "Hash": "64",
                "SearchMode": "Speculative", "SpecTopN": "0",
                "MoveOverhead": "0", "NnueOutput": "absolute", "FV_SCALE": "24"}
            summary[f"engine{arm}_eval_file_acknowledgement"] = (
                f"info string NNUE weights loaded from {path}")
        (pair / "result.json").write_text(json.dumps(summary))
        (pair / "result.jsonl").write_text(
            '{"id":"pos0_pair0","result":"baseline_win"}\n'
            '{"id":"pos0_pair0","result":"candidate_win"}\n')
        return pair, plan, summary

    def test_command_pins_both_weights_and_every_option_pair(self):
        command = G.match_command(self.plan(), Path("/tmp/root"), Path("/tmp/pair"))
        self.assertEqual(command.count("--engine-option1"), 8)
        self.assertEqual(command.count("--engine-option2"), 8)
        self.assertNotIn("UseBook=false", command)
        for index, value in enumerate(command):
            if value.startswith("--engine-option"):
                self.assertIn("=", command[index + 1])
        self.assertIn("EvalFile=/tmp/hypothesis", command)
        self.assertIn("EvalFile=/tmp/control", command)

    def test_pair_requires_complete_two_game_acks_and_records(self):
        with tempfile.TemporaryDirectory() as temporary:
            pair, plan, summary = self.make_pair(Path(temporary))
            self.assertEqual(G.validate_pair(pair, plan),
                             ["baseline_win", "candidate_win"])
            summary["engine2_eval_file_acknowledgement"] = None
            (pair / "result.json").write_text(json.dumps(summary))
            with self.assertRaisesRegex(ValueError, "engine 2 identity"):
                G.validate_pair(pair, plan)

    def test_incomplete_pair_cannot_enter_aggregate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaisesRegex(ValueError, "incomplete aggregate pair"):
                G.write_aggregate(root, [["candidate_win"]])
            path = G.write_aggregate(root, [["candidate_win", "draw"]])
            summary = json.loads(path.read_text())
            self.assertEqual((summary["games"], summary["engine1_wins"],
                              summary["draws"]), (2, 1, 1))
            rows = [json.loads(line) for line in
                    (root / "combined.jsonl").read_text().splitlines()]
            self.assertEqual({row["id"] for row in rows}, {"pos0_pair0"})

    def test_modified_pinned_input_fails_before_game(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "weight"
            path.write_bytes(b"old")
            plan = {"pins": {"hypothesis": {"path": str(path),
                                           "sha256": G.digest(path)}}}
            G.check_pins(plan)
            path.write_bytes(b"new")
            with self.assertRaisesRegex(ValueError, "pin changed"):
                G.check_pins(plan)


if __name__ == "__main__":
    unittest.main()
