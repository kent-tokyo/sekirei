#!/usr/bin/env python3
"""Unit tests for gate_resource_preflight.py's pure parsing/decision logic.

All tests use synthetic strings/numbers -- none of them call uptime/sysctl/
vm_stat/df/pgrep or touch the real host. That's deliberate: this file only
exercises the parse_*/evaluate_*/build_checks functions, never run()/collect_*
(which are thin, untested-here wrappers around real subprocess calls).

No third-party dependencies (stdlib unittest only), matching this repo's
scripts/test_gate_dashboard.py convention.

Run: python3 scripts/test_gate_resource_preflight.py

(Not run as part of producing this PR -- see the PR body for why: the host
was under sustained CPU/swap pressure and GitHub CI is the first executable
validation for this change.)
"""
import unittest

from gate_resource_preflight import (
    Check,
    build_checks,
    evaluate_thread_budget,
    parse_contending_pids,
    parse_disk_free_gb,
    parse_free_memory_gb,
    parse_int,
    parse_load_average_1min,
    parse_pgrep_pids,
    parse_process_count,
    parse_process_present,
    parse_swap_used_fraction,
)

SAMPLE_UPTIME = "07:59  up 24 days, 20:34, 5 users, load averages: 17.51 15.32 14.20\n"

SAMPLE_SWAPUSAGE = "vm.swapusage: total = 5120.00M  used = 4200.00M  free = 920.00M  (encrypted)\n"
SAMPLE_SWAPUSAGE_LOW = "vm.swapusage: total = 5120.00M  used = 512.00M  free = 4608.00M  (encrypted)\n"

SAMPLE_VM_STAT_OK = (
    "Mach Virtual Memory Statistics: (page size of 4096 bytes)\n"
    "Pages free:                             524288.\n"
    "Pages active:                           100000.\n"
)
SAMPLE_VM_STAT_LOW = (
    "Mach Virtual Memory Statistics: (page size of 4096 bytes)\n"
    "Pages free:                                100.\n"
    "Pages active:                           900000.\n"
)

SAMPLE_DF_OK = (
    "Filesystem     1G-blocks  Used Available Capacity  Mounted on\n"
    "/dev/disk3s1s1       926   601       300    67%    /\n"
)
SAMPLE_DF_LOW = (
    "Filesystem     1G-blocks  Used Available Capacity  Mounted on\n"
    "/dev/disk3s1s1       926   920         6    99%    /\n"
)

SAMPLE_PGREP_HIT = "1234 renkin-crowdout-diag\n5678 renkin-crowdout-diag --flag\n"
SAMPLE_PGREP_MISS = ""  # pgrep ran, found nothing -- a real, known "no match"
SAMPLE_PGREP_CLAUDE_5 = "111\n222\n333\n444\n555\n"


class ParseLoadAverageTests(unittest.TestCase):
    def test_normal_output(self):
        self.assertEqual(parse_load_average_1min(SAMPLE_UPTIME), 17.51)

    def test_none_input(self):
        self.assertIsNone(parse_load_average_1min(None))

    def test_unparseable_output(self):
        self.assertIsNone(parse_load_average_1min("totally unexpected output\n"))


class ParseSwapFractionTests(unittest.TestCase):
    def test_high_swap(self):
        frac = parse_swap_used_fraction(SAMPLE_SWAPUSAGE)
        self.assertAlmostEqual(frac, 4200.0 / 5120.0)

    def test_low_swap(self):
        frac = parse_swap_used_fraction(SAMPLE_SWAPUSAGE_LOW)
        self.assertAlmostEqual(frac, 512.0 / 5120.0)

    def test_none_input(self):
        self.assertIsNone(parse_swap_used_fraction(None))

    def test_unparseable_output(self):
        self.assertIsNone(parse_swap_used_fraction("vm.swapusage: garbage\n"))

    def test_zero_total_does_not_divide_by_zero(self):
        self.assertIsNone(
            parse_swap_used_fraction("vm.swapusage: total = 0.00M  used = 0.00M  free = 0.00M\n")
        )


class ParseFreeMemoryTests(unittest.TestCase):
    def test_normal_output_exact_2gb(self):
        # 4096 bytes/page * 524288 pages = 2147483648 bytes = exactly 2.0 GiB
        self.assertAlmostEqual(parse_free_memory_gb(SAMPLE_VM_STAT_OK), 2.0)

    def test_low_memory(self):
        gb = parse_free_memory_gb(SAMPLE_VM_STAT_LOW)
        self.assertLess(gb, 0.001)

    def test_none_input(self):
        self.assertIsNone(parse_free_memory_gb(None))

    def test_missing_fields(self):
        self.assertIsNone(parse_free_memory_gb("nothing useful here\n"))


class ParseDiskFreeTests(unittest.TestCase):
    def test_normal_output(self):
        self.assertAlmostEqual(parse_disk_free_gb(SAMPLE_DF_OK), 300.0)

    def test_low_disk(self):
        self.assertAlmostEqual(parse_disk_free_gb(SAMPLE_DF_LOW), 6.0)

    def test_none_input(self):
        self.assertIsNone(parse_disk_free_gb(None))

    def test_too_few_lines(self):
        self.assertIsNone(parse_disk_free_gb("Filesystem only, no data row\n"))


class ParseProcessPresentTests(unittest.TestCase):
    def test_hit(self):
        self.assertTrue(parse_process_present(SAMPLE_PGREP_HIT))

    def test_miss_is_false_not_unknown(self):
        # pgrep ran and legitimately found nothing -- a known "no", not "unknown".
        self.assertFalse(parse_process_present(SAMPLE_PGREP_MISS))

    def test_none_input_is_unknown(self):
        self.assertIsNone(parse_process_present(None))


class ParsePgrepPidsTests(unittest.TestCase):
    def test_extracts_pids_from_pgrep_fl_output(self):
        raw = "47087 npm exec @upstash/context7-mcp\n52012 cargo build --release\n"
        self.assertEqual(parse_pgrep_pids(raw), [47087, 52012])

    def test_empty_output_is_empty_list_not_unknown(self):
        self.assertEqual(parse_pgrep_pids(""), [])

    def test_none_input_is_unknown(self):
        self.assertIsNone(parse_pgrep_pids(None))


class ParseContendingPidsTests(unittest.TestCase):
    # 2026-08-27: a 6-day-old, idle `npm exec @upstash/context7-mcp` process
    # matched --contention-job renkin via `pgrep -f` (its cwd contained the
    # substring) and refused launch for days despite consuming ~0% CPU, while
    # the actual heavy build it was meant to detect had already finished.
    # These tests cover the CPU-threshold filter added to fix that.

    def test_process_above_threshold_is_contending(self):
        raw = "47087  87.3\n"
        self.assertEqual(parse_contending_pids(raw, threshold_percent=5.0), [47087])

    def test_idle_process_below_threshold_is_not_contending(self):
        # The exact false-positive shape: present, but ~0% CPU.
        raw = "47087   0.0\n"
        self.assertEqual(parse_contending_pids(raw, threshold_percent=5.0), [])

    def test_mixed_pids_filters_to_only_contending_ones(self):
        raw = "47087   0.0\n52012  63.8\n"
        self.assertEqual(parse_contending_pids(raw, threshold_percent=5.0), [52012])

    def test_empty_output_is_empty_list_not_unknown(self):
        self.assertEqual(parse_contending_pids("", threshold_percent=5.0), [])

    def test_none_input_is_unknown(self):
        self.assertIsNone(parse_contending_pids(None, threshold_percent=5.0))

    def test_malformed_line_is_skipped_not_fatal(self):
        raw = "not a valid ps line\n52012  63.8\n"
        self.assertEqual(parse_contending_pids(raw, threshold_percent=5.0), [52012])


class ParseProcessCountTests(unittest.TestCase):
    def test_counts_lines(self):
        self.assertEqual(parse_process_count(SAMPLE_PGREP_CLAUDE_5), 5)

    def test_empty_is_zero_not_unknown(self):
        self.assertEqual(parse_process_count(SAMPLE_PGREP_MISS), 0)

    def test_none_input_is_unknown(self):
        self.assertIsNone(parse_process_count(None))


class ParseIntTests(unittest.TestCase):
    def test_valid(self):
        self.assertEqual(parse_int("10\n"), 10)

    def test_none(self):
        self.assertIsNone(parse_int(None))

    def test_garbage(self):
        self.assertIsNone(parse_int("not a number\n"))


class EvaluateThreadBudgetTests(unittest.TestCase):
    def test_within_limit_at_boundary(self):
        # 1 shard x 2 engines x (Threads=1 + spec_top_n=3) = 8, limit = 10-2 = 8
        # -- equal to the limit counts as OK (matches the live preflight run
        # observed during this session's audit, which reported PASS at
        # exactly this configuration).
        predicted, limit, ok = evaluate_thread_budget(
            parallel=1, threads=1, spec_top_n=3, physical_cores=10
        )
        self.assertEqual(predicted, 8)
        self.assertEqual(limit, 8)
        self.assertTrue(ok)

    def test_over_limit(self):
        # 6 shards x 2 engines x (Threads=2 + spec_top_n=3) = 60, limit = 8
        predicted, limit, ok = evaluate_thread_budget(
            parallel=6, threads=2, spec_top_n=3, physical_cores=10
        )
        self.assertEqual(predicted, 60)
        self.assertEqual(limit, 8)
        self.assertFalse(ok)

    def test_unknown_cores_gives_unknown_verdict_not_true(self):
        predicted, limit, ok = evaluate_thread_budget(
            parallel=1, threads=1, spec_top_n=3, physical_cores=None
        )
        self.assertEqual(predicted, 8)  # still computable
        self.assertIsNone(limit)
        self.assertIsNone(ok)  # NOT True -- unknown cores must never look safe


class CheckTests(unittest.TestCase):
    def test_pass_status(self):
        c = Check("x", True, "detail")
        self.assertEqual(c.status, "PASS")
        self.assertTrue(c.passed)

    def test_refuse_status(self):
        c = Check("x", False, "detail")
        self.assertEqual(c.status, "REFUSE")
        self.assertFalse(c.passed)

    def test_unknown_status_is_not_passed(self):
        c = Check("x", None, "detail")
        self.assertEqual(c.status, "UNKNOWN")
        self.assertFalse(c.passed)  # unknown must never count as passed


class BuildChecksTests(unittest.TestCase):
    def test_all_clear_scenario_passes(self):
        checks = build_checks(
            physical_cores=10,
            logical_cores=10,
            load1=2.0,
            swap_fraction=0.05,
            free_mem_gb=8.0,
            disk_free_gb_value=50.0,
            contention_hits=[],
            claude_session_count_value=1,
            parallel=1,
            threads=1,
            spec_top_n=3,
        )
        self.assertTrue(all(c.passed for c in checks))

    def test_contended_scenario_refuses(self):
        # Mirrors the real conditions observed during this session's audit:
        # load ~17, swap ~82%, a renkin-crowdout-diag hit, 4+ claude sessions.
        checks = build_checks(
            physical_cores=10,
            logical_cores=10,
            load1=17.51,
            swap_fraction=0.82,
            free_mem_gb=0.08,
            disk_free_gb_value=25.0,
            contention_hits=["renkin"],
            claude_session_count_value=4,
            parallel=1,
            threads=1,
            spec_top_n=3,
        )
        self.assertFalse(all(c.passed for c in checks))
        by_label = {c.label: c for c in checks}
        self.assertEqual(by_label["load average (1min)"].status, "REFUSE")
        self.assertEqual(by_label["swap used fraction"].status, "REFUSE")
        self.assertEqual(by_label["free memory (GB)"].status, "REFUSE")
        self.assertEqual(by_label["named contention jobs"].status, "REFUSE")
        self.assertEqual(by_label["concurrent claude sessions"].status, "REFUSE")
        # disk was fine in this scenario -- must still show PASS, not get
        # dragged down by the other failing checks.
        self.assertEqual(by_label["disk free (GB)"].status, "PASS")

    def test_unknown_value_shows_unknown_not_pass(self):
        checks = build_checks(
            physical_cores=None,  # e.g. sysctl unavailable
            logical_cores=None,
            load1=None,
            swap_fraction=0.05,
            free_mem_gb=8.0,
            disk_free_gb_value=50.0,
            contention_hits=None,  # pgrep itself failed
            claude_session_count_value=1,
            parallel=1,
            threads=1,
            spec_top_n=3,
        )
        by_label = {c.label: c for c in checks}
        self.assertEqual(by_label["physical cores"].status, "UNKNOWN")
        self.assertEqual(by_label["load average (1min)"].status, "UNKNOWN")
        self.assertEqual(by_label["named contention jobs"].status, "UNKNOWN")
        # predicted-threads check depends on physical_cores too -> also unknown
        self.assertEqual(by_label["predicted CPU-competing threads"].status, "UNKNOWN")
        self.assertFalse(all(c.passed for c in checks))


if __name__ == "__main__":
    unittest.main()
