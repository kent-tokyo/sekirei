#!/usr/bin/env python3

import copy
import json
import unittest
from pathlib import Path

from validate_search_mode_gate import validate


PLAN = json.loads(
    Path("results/search_mode_strength_gate_plan_20260909.json").read_text(
        encoding="utf-8"
    )
)


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
