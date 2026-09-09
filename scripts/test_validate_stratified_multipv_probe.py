#!/usr/bin/env python3
"""Regression tests for the stratified MultiPV report validator."""

import unittest

from validate_stratified_multipv_probe import validate


def report(lines=None):
    return {
        "schema_version": 1,
        "diagnostic_only": True,
        "positions": [{
            "sample_id": "p1",
            "evaluators": {"candidate": {"status": "ok", "lines": lines or [{"multipv": 1, "move": "7g7f", "score_cp": 12}]}, "teacher": {"status": "incomplete"}, "material": {"status": "incomplete"}},
        }],
    }


class ValidatorTest(unittest.TestCase):
    def test_valid_report(self):
        self.assertEqual(validate(report()), [])

    def test_rejects_duplicate_multipv(self):
        self.assertTrue(validate(report([{"multipv": 1, "move": "7g7f", "score_cp": 12}, {"multipv": 1, "move": "2g2f", "score_cp": 8}])))


if __name__ == "__main__":
    unittest.main()
