#!/usr/bin/env python3

import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from validate_search_mode_gate import validate


PLAN = {
    "schema": "sekirei.search-mode-strength-gate-plan.v1",
    "purpose": "isolate SearchMode implementation effects with identical binary and weights",
    "status": "planned",
    "positions": 99,
    "games_per_position": 2,
    "byoyomi_ms": 100,
    "max_games": 198,
    "engine1": {
        "Threads": "1",
        "SpecTopN": "0",
        "UseBook": "false",
        "SearchMode": "Speculative",
    },
    "engine2": {
        "Threads": "1",
        "SpecTopN": "0",
        "UseBook": "false",
        "SearchMode": "LazySMP",
    },
}


class SearchModePlanTests(unittest.TestCase):
    def test_checked_in_plan_is_valid(self):
        self.assertEqual(validate(PLAN), [])

    def test_mixed_threads_are_rejected(self):
        plan = copy.deepcopy(PLAN)
        plan["engine2"]["Threads"] = "2"
        self.assertTrue(validate(plan))

    def test_same_mode_is_rejected(self):
        plan = copy.deepcopy(PLAN)
        plan["engine2"]["SearchMode"] = "Speculative"
        self.assertTrue(validate(plan))


if __name__ == "__main__":
    unittest.main()
