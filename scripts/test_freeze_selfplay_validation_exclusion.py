#!/usr/bin/env python3
"""Focused tests for canonical validation exclusions."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("exclude", ROOT / "freeze_selfplay_validation_exclusion.py")
assert SPEC and SPEC.loader
EXCLUDE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EXCLUDE)


class ExclusionTests(unittest.TestCase):
    def test_canonical_sfen_drops_only_move_number(self) -> None:
        self.assertEqual(
            EXCLUDE.canonical_sfen("lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"),
            EXCLUDE.canonical_sfen("lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 99"),
        )


if __name__ == "__main__":
    unittest.main()
