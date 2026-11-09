#!/usr/bin/env python3
"""Boundary checks for the proposed matched HalfKP resource contract."""
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts import matched_nnue_gate_capacity as G


class MatchedGateCapacityTests(unittest.TestCase):
    def setUp(self):
        self.policy = G.CapacityPolicy()
        self.good = {"observation_complete": True, "local_free_mib": 16384,
                     "archive_free_mib": 8192,
                     "local_and_archive_same_device": False}

    def test_exact_start_thresholds_and_missing_telemetry(self):
        self.assertTrue(G.start_sample_ok(self.good, self.policy))
        for key, value in (("local_free_mib", 16383), ("archive_free_mib", 8191),
                           ("local_free_mib", True), ("observation_complete", False),
                           ("local_and_archive_same_device", None)):
            with self.subTest(key=key, value=value):
                self.assertFalse(G.start_sample_ok(dict(self.good, **{key: value}), self.policy))

    def test_stability_resets_and_requires_elapsed_time(self):
        window = None
        for at in range(10, 610, 10):
            window = G.advance_window(window, at, self.good, self.policy)
        self.assertFalse(G.start_admitted(window, 609, self.good, self.policy))
        window = G.advance_window(window, 610, self.good, self.policy)
        self.assertTrue(G.start_admitted(window, 610, self.good, self.policy))
        bad = dict(self.good, archive_free_mib=8191)
        window = G.advance_window(window, 620, bad, self.policy)
        self.assertIsNone(window)
        window = G.advance_window(window, 630, self.good, self.policy)
        self.assertEqual(window, G.StableWindow(630, 630))
        self.assertFalse(G.start_admitted(window, 630, self.good, self.policy))
        with self.assertRaises(ValueError):
            G.advance_window(window, 629, self.good, self.policy)

    def test_missed_poll_restarts_stability_clock(self):
        window = G.advance_window(None, 10, self.good, self.policy)
        self.assertEqual(G.advance_window(window, 26, self.good, self.policy),
                         G.StableWindow(26, 26))

    def test_after_pair_never_treats_missing_or_low_space_as_safe(self):
        self.assertEqual(G.after_pair_action(self.good, 1024**3, self.policy),
                         "continue_if_other_gates_pass")
        self.assertEqual(G.after_pair_action(dict(self.good, local_free_mib=8192),
                                             1024**3, self.policy),
                         "continue_if_other_gates_pass")
        for sample, size in ((dict(self.good, local_free_mib=8191), 0),
                             (dict(self.good, observation_complete=False), 0),
                             (self.good, 1024**3 + 1), (self.good, -1),
                             (self.good, None)):
            self.assertEqual(G.after_pair_action(sample, size, self.policy),
                             "stop_after_retaining_pair")

    def test_missing_mount_path_is_unknown(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(G.os.path, "ismount", return_value=True):
                self.assertFalse(G.observe(root, root / "missing", root)["observation_complete"])
                observed = G.observe(root, root, root)
            self.assertTrue(observed["observation_complete"])
            self.assertTrue(observed["local_and_archive_same_device"])
            self.assertFalse(G.start_sample_ok(observed, self.policy))

    def test_unmounted_archive_path_fails_even_if_directory_exists(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(G.os.path, "ismount", return_value=False):
                self.assertFalse(G.observe(root, root, root)["observation_complete"])

    def test_invalid_policy(self):
        for kwargs in ({"start_local_free_mib": 8192}, {"stable_seconds": 0},
                       {"output_cap_bytes": True}, {"max_sample_gap_seconds": 0}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                G.CapacityPolicy(**kwargs)


if __name__ == "__main__":
    unittest.main()
