#!/usr/bin/env python3
"""Unit tests for run_parallel_sprt.py (stdlib unittest only).

Run: python3 scripts/test_run_parallel_sprt.py
"""

import json
import stat
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_parallel_sprt import (  # noqa: E402
    main,
    merge_options,
    pair_score,
    parse_args,
    validate_pair_summary,
)

FAKE_MATCH = textwrap.dedent("""\
    #!/usr/bin/env python3
    import json, os, sys
    argv = sys.argv[1:]
    out = argv[argv.index("--json") + 1]
    wins = int(os.environ.get("FAKE_E1_WINS", "2"))
    status = os.environ.get("FAKE_STATUS", "complete")
    summary = {"status": status, "games": 2, "engine1_wins": wins,
               "draws": 0, "engine2_wins": 2 - wins, "invalid_games": []}
    with open(out, "w") as handle:
        json.dump(summary, handle)
    """)


class HelperTests(unittest.TestCase):
    def test_merge_options_defaults_and_overrides(self):
        merged = merge_options(["Hash=256", "EvalFile=/a"], ["FV_SCALE=24", "Hash=128"])
        self.assertEqual(merged, ["Threads=1", "FV_SCALE=24", "Hash=256", "EvalFile=/a"])

    def test_merge_options_usi_hash_replaces_default_hash(self):
        self.assertEqual(merge_options(["USI_Hash=64"], []), ["Threads=1", "USI_Hash=64"])
        self.assertIn("Hash=32", merge_options(["USI_Hash=64", "Hash=32"], []))

    def test_merge_options_rejects_missing_value(self):
        with self.assertRaises(ValueError):
            merge_options(["Threads"], [])

    def test_pair_summary_validation(self):
        good = {"status": "complete", "games": 2, "invalid_games": []}
        self.assertIsNone(validate_pair_summary(good))
        self.assertIn("status", validate_pair_summary({**good, "status": "running"}))
        self.assertIn("2 games", validate_pair_summary({**good, "games": 1}))
        self.assertIn("invalid", validate_pair_summary({**good, "invalid_games": ["g1"]}))

    def test_pair_score(self):
        self.assertEqual(pair_score({"engine1_wins": 1, "draws": 1}), 1.5)


class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.match = root / "fake-match"
        self.match.write_text(FAKE_MATCH)
        self.match.chmod(self.match.stat().st_mode | stat.S_IEXEC)
        self.engine = root / "engine"
        self.engine.write_text("")
        self.positions = root / "positions.sfen"
        self.positions.write_text("startpos\n# comment\nstartpos moves 7g7f\n")
        self.out = root / "out"

    def tearDown(self):
        self.tmp.cleanup()

    def argv(self, *extra):
        return ["--engine1", str(self.engine), "--engine2", str(self.engine),
                "--positions", str(self.positions), "--match-binary", str(self.match),
                "--out-dir", str(self.out), "--concurrency", "2", *extra]

    def summary(self):
        return json.loads((self.out / "summary.json").read_text())

    def test_fixed_pairs_complete(self):
        import os
        os.environ["FAKE_E1_WINS"] = "1"
        try:
            self.assertEqual(main(self.argv("--pairs", "4")), 0)
        finally:
            del os.environ["FAKE_E1_WINS"]
        summary = self.summary()
        self.assertEqual(summary["verdict"], "complete")
        self.assertEqual(summary["pairs_completed"], 4)
        self.assertEqual(summary["pair_counts"], {"win": 0, "draw": 4, "loss": 0})
        self.assertEqual(summary["positions"]["count"], 2)
        records = [json.loads(line) for line in (self.out / "pairs.jsonl").read_text().splitlines()]
        self.assertEqual(sorted(r["position_index"] for r in records), [0, 0, 1, 1])

    def test_sprt_stops_early_on_h1(self):
        self.assertEqual(main(self.argv("--pairs", "1000", "--sprt", "0,200")), 0)
        summary = self.summary()
        self.assertEqual(summary["verdict"], "H1_accepted")
        self.assertLess(summary["pairs_completed"], 1000)
        self.assertGreaterEqual(summary["sprt"]["llr"], summary["sprt"]["upper"])

    def test_errors_abort(self):
        import os
        os.environ["FAKE_STATUS"] = "running"
        try:
            self.assertEqual(main(self.argv("--pairs", "50", "--max-errors", "1")), 1)
        finally:
            del os.environ["FAKE_STATUS"]
        summary = self.summary()
        self.assertEqual(summary["verdict"], "aborted_errors")
        self.assertEqual(summary["pairs_completed"], 0)

    def test_rejects_non_empty_out_dir(self):
        self.out.mkdir()
        (self.out / "x").write_text("")
        with self.assertRaises(SystemExit):
            parse_args(self.argv("--pairs", "1"))


if __name__ == "__main__":
    unittest.main()
