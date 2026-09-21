#!/usr/bin/env python3
"""Focused tests for the Q21h independent split freezer."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("q21h", ROOT / "freeze_q21h_independent_split.py")
assert SPEC and SPEC.loader
Q21H = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(Q21H)


START = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"


class FreezeQ21hIndependentSplitTests(unittest.TestCase):
    def test_symmetry_key_groups_mirror_and_color_rotation(self) -> None:
        mirrored = Q21H.transform_sfen(START, mirror=True, rotate_swap=False)
        rotated = Q21H.transform_sfen(START, mirror=False, rotate_swap=True)
        self.assertEqual(Q21H.symmetry_key(START), Q21H.symmetry_key(mirrored))
        self.assertEqual(Q21H.symmetry_key(START), Q21H.symmetry_key(rotated))

    def test_hand_order_is_canonical_after_color_rotation(self) -> None:
        position = "9/9/9/9/4k4/9/9/9/4K4 b R2Prb 1"
        rotated = Q21H.transform_sfen(position, mirror=False, rotate_swap=True)
        self.assertEqual(rotated.split()[2], "RBr2p")

    def test_replay_phase_checks_absolute_move_number(self) -> None:
        row = {"ply": 20, "pre_move_sfen": START.rsplit(" ", 1)[0] + " 21"}
        self.assertEqual(Q21H.replay_phase(START, row), ("middlegame", 21))
        row["pre_move_sfen"] = START.rsplit(" ", 1)[0] + " 22"
        with self.assertRaises(ValueError):
            Q21H.replay_phase(START, row)

    def test_select_respects_per_game_cap_and_is_deterministic(self) -> None:
        rows = []
        for source in ("a", "b"):
            for index in range(5):
                rows.append({
                    "sfen": f"9/9/9/9/9/9/9/9/{index + 1}K{8 - index} b - {index + 1}",
                    "identity": f"{source}-{index}",
                    "source": {"source_key": source},
                    "tags": {"phase": "opening", "material_band": "balanced"},
                })
        first = Q21H.select(rows, limit=10, source_cap=2, seed=42)
        second = Q21H.select(list(reversed(rows)), limit=10, source_cap=2, seed=42)
        self.assertEqual([row["identity"] for row in first], [row["identity"] for row in second])
        self.assertEqual(len(first), 4)
        self.assertEqual(max(__import__("collections").Counter(row["source"]["source_key"] for row in first).values()), 2)

    def test_mate_status_does_not_invent_kind_from_numeric_cp(self) -> None:
        self.assertEqual(Q21H.mate_status({"observed_score_cp": 32000}), "unknown")
        self.assertEqual(Q21H.mate_status({"score_kind": "mate"}), "mate")
        self.assertEqual(Q21H.mate_status({"score_kind": "cp"}), "non_mate")

    def test_group_assignment_covers_rare_validation_stratum_without_splitting_group(self) -> None:
        groups = {"common": ["a"], "rare": ["b"]}
        rows = []
        for group, material in (("common", "balanced"), ("rare", "stm_ahead")):
            for index in range(20):
                rows.append({
                    "source": {"derived_group": group, "source_key": f"{group}-{index % 4}"},
                    "tags": {"phase": "opening", "material_band": material},
                })
        assignment = Q21H.assign_groups(groups, rows, seed=42)
        self.assertEqual(assignment["common"], "validation")
        self.assertEqual(assignment["rare"], "validation")


if __name__ == "__main__":
    unittest.main()
