#!/usr/bin/env python3

import sys
import unittest
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import freeze_q21g_teacher_corpus as subject


def candidate(index: int, selection_class: str, phase: str, material: str, source: str):
    return {
        "id": f"p{index:03d}",
        "selection_class": selection_class,
        "source": {"game_id": source},
        "attributes": {"phase": phase, "material_band": material},
        "tactical_flags": {"capture": selection_class == "tactical"},
        "selection_rank": f"{index:064x}",
    }


class FreezeQ21gCorpusTests(unittest.TestCase):
    def test_tactical_flags_keep_motifs_separate(self):
        position = {
            "side_to_move_in_check": True,
            "captured_piece": "FU",
            "actual_move_is_drop": False,
        }
        next_position = {"side_to_move_in_check": True}
        flags = subject.tactical_flags(position, next_position, "7c7b+")
        self.assertEqual(
            flags,
            {
                "in_check": True,
                "gives_check": True,
                "king_threat": True,
                "capture": True,
                "promotion": True,
                "drop": False,
            },
        )

    def test_freeze_covers_all_strata_and_caps_sources(self):
        rows = []
        index = 0
        for selection_class in ("normal", "tactical"):
            for phase, material in subject.STRATA:
                for repetition in range(3):
                    rows.append(
                        candidate(
                            index,
                            selection_class,
                            phase,
                            material,
                            f"g-{selection_class}-{phase}-{material}-{repetition}",
                        )
                    )
                    index += 1
        frozen = subject.freeze(rows, 18, 9)
        self.assertEqual(len(frozen), 27)
        self.assertEqual(Counter(row["selection_class"] for row in frozen), {"normal": 18, "tactical": 9})
        self.assertEqual(
            {(row["attributes"]["phase"], row["attributes"]["material_band"]) for row in frozen},
            set(subject.STRATA),
        )
        self.assertLessEqual(
            max(Counter(row["source"]["game_id"] for row in frozen).values()),
            2,
        )

    def test_counts_reports_missing_stratum(self):
        row = candidate(1, "normal", "opening", "balanced", "g1")
        summary = subject.counts([row])
        self.assertEqual(summary["positions"], 1)
        self.assertIn("endgame:stm_ahead", summary["missing_phase_material"])


if __name__ == "__main__":
    unittest.main()
