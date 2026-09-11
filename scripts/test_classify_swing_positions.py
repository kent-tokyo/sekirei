#!/usr/bin/env python3
import importlib.util
import tempfile
import unittest

ROOT = __import__("pathlib").Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("classify_swing_positions", ROOT / "classify_swing_positions.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class SwingPositionTest(unittest.TestCase):
    def test_material_and_negative_filter(self):
        sfen = "9/9/9/9/9/9/9/9/R8 b - 1"
        batch = {"reports": [{"game_id": "g", "result": "lose", "records": [
            {"sfen": sfen, "swing": True, "terminal": False, "score_delta_cp": -250, "ply": 4},
            {"sfen": sfen, "swing": True, "terminal": True, "score_delta_cp": -900000, "ply": 6},
            {"sfen": sfen, "swing": True, "terminal": False, "score_delta_cp": 250, "ply": 8},
        ]}]}
        with tempfile.TemporaryDirectory() as directory:
            csa = ROOT / "_test_game.csa"
            csa.write_text("+7776FU\n-3334FU\n+2726FU\n-8384FU\n", encoding="utf-8")
            batch["reports"][0]["csa"] = str(csa)
            result = MODULE.classify(batch)
            csa.unlink()
        self.assertEqual(len(result["positions"]), 1)
        self.assertEqual(result["positions"][0]["our_material_cp"], 1000)
        self.assertEqual(result["positions"][0]["material_bucket"], "ahead")
        self.assertEqual(result["positions"][0]["previous_move_csa"], "-8384FU")
        self.assertEqual(result["positions"][0]["two_moves_back_csa"], "+2726FU")


if __name__ == "__main__":
    unittest.main()
