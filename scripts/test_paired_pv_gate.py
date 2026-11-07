#!/usr/bin/env python3
"""Completion gates with synthetic engine/checker responses; no timed searches."""
import json
import subprocess
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch

import paired_speculative_diagnostics as driver
import validate_paired_pv as validator
from test_paired_speculative_diagnostics import IDLE, legal_probe, legal_pv


def slot(_binary, root, _config, _movetime, _timeout, _directory):
    return {"depth": 2, "nodes": 10, "elapsed_ms": 301,
            "response_overrun_ms": 1, **legal_pv(root)}


class CompletionGate(unittest.TestCase):
    def fixture(self, root, corpus="startpos\nstartpos moves 7g7f\n"):
        binary, positions, legal = root / "engine-source", root / "roots", root / "checker-source"
        binary.write_bytes(b"synthetic search executable; never execute")
        legal.write_bytes(b"synthetic Sekirei checker; never execute")
        positions.write_text(corpus)
        return ["--binary", str(binary), "--positions", str(positions),
                "--legal-moves", str(legal), "--output", str(root / "out"),
                "--repeats", "1", "--resource-root", str(root), "--minimum-free-mib", "1"]

    def run_mock(self, args, *, slot_fn=slot, probe_fn=legal_probe, validate_fn=None):
        with ExitStack() as stack:
            stack.enter_context(patch.object(driver, "run_slot", side_effect=slot_fn))
            stack.enter_context(patch.object(driver, "host_activity", return_value=IDLE))
            stack.enter_context(patch.object(validator, "probe", side_effect=probe_fn))
            if validate_fn is not None:
                stack.enter_context(patch.object(validator, "validate_runs", side_effect=validate_fn))
            return driver.main(args)

    def receipt(self, root):
        return json.loads((root / "out/manifest.json").read_text())

    def assert_disqualified(self, root, *, report=True):
        manifest = self.receipt(root)
        self.assertEqual(manifest["status"], "incomplete")
        self.assertFalse(manifest["timing_qualified"])
        self.assertIs(manifest["pv_validation"]["valid"], False)
        if report:
            self.assertEqual(manifest["pv_validation"]["sha256"], driver.sha256(root / "out/pv-validation.json"))
            summary = json.loads((root / "out/summary.json").read_text())
            self.assertFalse(summary["performance_comparison_eligible"])
            self.assertNotIn("pv_validation_sha256", summary)
        return manifest

    def test_complete_run_binds_every_root_history_pv_and_receipt_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # Preserve caller schedule order even though JSON object keys sort.
            self.assertEqual(self.run_mock(self.fixture(root) + ["--threads", "8,4"]), 0)
            manifest = self.receipt(root)
            self.assertEqual(manifest["schema"], "sekirei.speculative-paired-diagnostic.v2.manifest")
            self.assertEqual(manifest["worker_order"], [8, 4])
            self.assertEqual(manifest["status"], "complete")
            self.assertTrue(manifest["timing_qualified"])
            self.assertTrue(manifest["pv_validation"]["valid"])
            self.assertFalse(manifest["legal_moves"]["build_identity_verified"])
            report = json.loads((root / "out/pv-validation.json").read_text())
            self.assertEqual(report["schema"], "sekirei.paired-pv-validation.v2")
            self.assertEqual((report["rows"], report["pv_moves"], report["root_history_moves"]), (16, 24, 8))
            record = report["runs"][0]
            self.assertTrue(record["strict_row_identity_verified"])
            self.assertEqual(record["rows_sha256"], driver.sha256(root / "out/rows.jsonl"))
            self.assertEqual(record["engine_sha256"], {side: manifest["engine"]["sha256"] for side in "AB"})
            self.assertEqual(record["corpus_sha256"], manifest["positions_sha256"])
            self.assertEqual(report["legal_tool_sha256"], manifest["legal_moves"]["sha256"])
            self.assertEqual(driver.sha256(Path(record["source_manifest_snapshot"])), record["source_manifest_sha256"])
            self.assertEqual(json.loads(Path(record["source_manifest_snapshot"]).read_text())["status"], "validating")
            self.assertEqual(manifest["pv_validation"]["sha256"], driver.sha256(root / "out/pv-validation.json"))
            summary = json.loads((root / "out/summary.json").read_text())
            self.assertTrue(summary["performance_comparison_eligible"])
            self.assertEqual(summary["pv_validation_sha256"], manifest["pv_validation"]["sha256"])

    def test_illegal_pv_never_becomes_complete_even_when_host_is_idle(self):
        def illegal(*args):
            return {**slot(*args), "bestmove": "7g7e", "pv": ["7g7e"]}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaisesRegex(ValueError, "legal-PV validation failed"):
                self.run_mock(self.fixture(root, "startpos\n"), slot_fn=illegal)
            self.assert_disqualified(root)
            report = json.loads((root / "out/pv-validation.json").read_text())
            self.assertTrue(report["violations"])
            self.assertEqual({item["phase"] for item in report["violations"]}, {"pv"})

    def test_illegal_root_history_fails_even_when_its_pv_is_legal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaisesRegex(ValueError, "legal-PV validation failed"):
                self.run_mock(self.fixture(root, "startpos moves 7g7e\n"))
            self.assert_disqualified(root)
            report = json.loads((root / "out/pv-validation.json").read_text())
            self.assertEqual(report["root_history_moves"], 4)
            self.assertEqual({item["phase"] for item in report["violations"]}, {"root_history"})

    def test_later_root_history_move_is_checked_before_a_legal_pv(self):
        def later_root(*args):
            return {**slot(*args), "bestmove": "2g2f", "pv": ["2g2f"]}
        def checker(_binary, boards):
            return ["7g7f 2g2f" if board.split()[1] == "b" else "3c3d" for board in boards]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaisesRegex(ValueError, "legal-PV validation failed"):
                self.run_mock(self.fixture(root, "startpos moves 7g7f 3c3e\n"), slot_fn=later_root, probe_fn=checker)
            self.assert_disqualified(root)
            report = json.loads((root / "out/pv-validation.json").read_text())
            self.assertEqual(report["root_history_moves"], 8)
            self.assertEqual({(item["phase"], item["ply"]) for item in report["violations"]}, {("root_history", 1)})

    def test_legacy_v1_validation_does_not_rewrite_historical_qualification(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "positions.input").write_text("startpos moves 7g7f\n")
            (root / "engine").write_bytes(b"synthetic frozen engine")
            (root / "checker").write_bytes(b"synthetic checker")
            legacy = {"schema": "sekirei.speculative-paired-diagnostic.v1.manifest", "status": "complete",
                      "timing_qualified": True, "positions_sha256": driver.sha256(root / "positions.input"),
                      "engine": {"path": str(root / "engine"), "sha256": driver.sha256(root / "engine")},
                      "expected_searches": 1, "completed_searches": 1}
            (root / "manifest.json").write_text(json.dumps(legacy))
            (root / "rows.jsonl").write_text(json.dumps({"sequence": 0, "position": 0, "bestmove": "3c3d", "pv": ["3c3d"]}) + "\n")
            original_manifest = (root / "manifest.json").read_bytes()
            with patch.object(validator, "probe", side_effect=legal_probe):
                result = validator.validate_runs(root / "checker", root / "new-validation.json", [root])
            self.assertTrue(result["valid"])
            self.assertEqual(result["schema"], validator.SCHEMA)
            self.assertFalse(result["runs"][0]["strict_row_identity_verified"])
            self.assertEqual((root / "manifest.json").read_bytes(), original_manifest)

    def test_absent_checker_blocks_before_engine_start_and_prepare_stays_unqualified(self):
        for prepare in (False, True):
            with self.subTest(prepare=prepare), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                args = self.fixture(root)
                index = args.index("--legal-moves")
                del args[index:index + 2]
                if prepare:
                    args.append("--prepare-only")
                with patch.object(driver, "run_slot") as run, patch.object(validator, "probe") as probe:
                    self.assertEqual(driver.main(args), 0 if prepare else 2)
                run.assert_not_called()
                probe.assert_not_called()
                manifest = self.receipt(root)
                self.assertEqual(manifest["status"], "prepared" if prepare else "blocked_validation")
                self.assertFalse(manifest["timing_qualified"])
                self.assertEqual(manifest["pv_validation"], {"status": "not_run", "valid": False})

    def test_missing_validator_output_cannot_be_treated_as_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaises(FileNotFoundError):
                self.run_mock(self.fixture(root), validate_fn=lambda *args, **kwargs: None)
            self.assert_disqualified(root, report=False)
            self.assertFalse((root / "out/pv-validation.json").exists())

    def test_false_unknown_or_truthy_non_boolean_reports_fail_closed(self):
        real = validator.validate_runs
        for verdict in (False, None, "unknown", 1):
            def altered(legal, output, runs, **kwargs):
                report = real(legal, output, runs, **kwargs)
                report["valid"] = verdict
                output.write_text(json.dumps(report))
                return report
            with self.subTest(verdict=verdict), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                with self.assertRaisesRegex(ValueError, "legal-PV validation failed"):
                    self.run_mock(self.fixture(root), validate_fn=altered)
                self.assert_disqualified(root)

    def test_checker_exit_timeout_partial_or_unknown_output_preserves_failure_receipt(self):
        for error in (subprocess.CalledProcessError(7, "mock-checker"),
                      subprocess.TimeoutExpired("mock-checker", 60), [], ["Unknown"]):
            def failed(_binary, boards):
                if isinstance(error, Exception):
                    raise error
                return error
            with self.subTest(error=repr(error)), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                with self.assertRaisesRegex(ValueError, "legal-PV validation failed"):
                    self.run_mock(self.fixture(root), probe_fn=failed)
                self.assert_disqualified(root)
                report = json.loads((root / "out/pv-validation.json").read_text())
                self.assertEqual(report["status"], "error")
                self.assertTrue(report["errors"])

    def test_partial_search_never_runs_validator_or_publishes_complete(self):
        calls = 0
        def interrupted(*args):
            nonlocal calls
            calls += 1
            if calls == 3:
                raise RuntimeError("synthetic interruption")
            return slot(*args)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(validator, "validate_runs") as validation:
                with self.assertRaisesRegex(RuntimeError, "synthetic interruption"):
                    self.run_mock(self.fixture(root), slot_fn=interrupted)
            validation.assert_not_called()
            manifest = self.receipt(root)
            self.assertEqual(manifest["completed_searches"], 2)
            self.assertEqual(manifest["status"], "incomplete")
            self.assertFalse(manifest["timing_qualified"])
            self.assertEqual(manifest["pv_validation"]["status"], "not_run")

    def test_partial_rows_and_changed_per_row_or_input_identity_cannot_qualify(self):
        real = validator.validate_runs
        for damage in ("partial", "sequence", "engine_sha256", "root_sha256", "configuration_sha256", "engine", "positions.input"):
            def altered(legal, output, runs, **kwargs):
                run = runs[0]
                if damage in ("engine", "positions.input"):
                    (run / damage).write_bytes(b"changed during validation")
                else:
                    rows = [json.loads(line) for line in (run / "rows.jsonl").read_text().splitlines()]
                    if damage == "partial":
                        rows.pop()
                    else:
                        rows[1][damage] = 0 if damage == "sequence" else "0" * 64
                    (run / "rows.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
                return real(legal, output, runs, **kwargs)
            with self.subTest(damage=damage), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                with self.assertRaisesRegex(ValueError, "legal-PV validation failed"):
                    self.run_mock(self.fixture(root), validate_fn=altered)
                self.assert_disqualified(root)

    def test_input_or_tool_drift_during_probe_is_rejected_after_replay(self):
        for damage in ("engine", "positions.input", "rows.jsonl", "legal-moves", "pv-validation.legal-moves"):
            with self.subTest(damage=damage), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                def drift(_binary, boards):
                    (root / "out" / damage).write_bytes(b"modified during probe")
                    return legal_probe(_binary, boards)
                with self.assertRaisesRegex(ValueError, "legal-PV validation failed"):
                    self.run_mock(self.fixture(root), probe_fn=drift)
                self.assert_disqualified(root)
                report = json.loads((root / "out/pv-validation.json").read_text())
                self.assertIn("changed during replay", report["errors"][0])

    def test_success_report_with_wrong_hash_counts_or_scope_does_not_qualify(self):
        real = validator.validate_runs
        for damage in ("rows_sha256", "source_manifest_sha256", "strict_row_identity_verified", "rows", "pv_moves", "root_history_moves", "legal_tool_sha256"):
            def altered(legal, output, runs, **kwargs):
                report = real(legal, output, runs, **kwargs)
                if damage in ("rows_sha256", "source_manifest_sha256", "strict_row_identity_verified"):
                    report["runs"][0][damage] = False if damage == "strict_row_identity_verified" else "0" * 64
                else:
                    report[damage] = "0" * 64 if damage == "legal_tool_sha256" else report[damage] - 1
                output.write_text(json.dumps(report))
                return report
            with self.subTest(damage=damage), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                with self.assertRaisesRegex(ValueError, "binding failed"):
                    self.run_mock(self.fixture(root), validate_fn=altered)
                self.assert_disqualified(root)


if __name__ == "__main__":
    unittest.main()
