#!/usr/bin/env python3
"""Synthetic TSVs and mocked native subprocesses only; never launches an engine."""
import copy
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import MagicMock, Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import produce_owned_teacher_observations as producer
import preflight_owned_teacher_stability as preflight
from test_preflight_owned_teacher_stability import A, B, C, fixture, tsv


def native_fields(sfen=A, depth=2, fv=24):
    move = "7a7b" if sfen == C else "5i5h"
    return {"bestmove": move, "depth": str(depth), "score_cp": str(depth*100), "nodes": "20",
            "elapsed_ns": "1000", "bound": "exact", "completed_bound": "exact",
            "completed_iteration_valid": "true", "aborted": "false", "abort_reason": "none",
            "teacher_search": "true", "fv_scale": str(fv), "pv_usi": move,
            "pv_legal": "true", "pv_replay_preserves_input": "true", "history_replayed": "false",
            "history_moves": "0", "history_matches_expected": "true"}


def native_text(fields):
    return "\t".join(key + "=" + value for key, value in fields.items()) + "\n"


def request_fixture(root):
    document, corpora, _ = fixture(root)
    document["schema"] = producer.SCHEMA
    document["attempt"] = {"reason": "synthetic contract test; no actual teacher execution", "previous_receipt": None}
    document["limits"] = {"per_position_seconds": .1, "cleanup_seconds": .1,
                           "max_wall_seconds": 10, "minimum_free_bytes": 1024**3, "max_positions": 16}
    search = document["provenance"]["teacher"]["search"]
    search.update(hash_mb=64, backend="search-diagnostic-v1", teacher_search=True, root_mate_safety=False,
                  qsearch={"source_commit": "1"*40, "configuration": "compiled-defaults"})
    for arm, content in corpora.items():
        path = root / (arm + ".tsv")
        path.write_text(content)
        document[arm] = {"path": path.name, "sha256": producer.file_sha256(path)}
    binary = root / "synthetic-binary-not-executable"
    binary.write_text("Mock binary identity, not an engine.\n")
    document["binary"] = {"path": binary.name, "sha256": producer.file_sha256(binary)}
    document["provenance"]["teacher"]["binary_sha256"] = document["binary"]["sha256"]
    return document


def mock_runner(binary, weights, request, sfen, depth, deadline, cwd):
    return producer.classify(native_text(native_fields(sfen, depth, request["scales"]["fv_scale"])),
                             sfen, depth, request["scales"]["fv_scale"]) | {
        "termination": {"exit_code": 0, "killed": False, "reaped": True}, "fixture": "mock_process"}


class NativeContractTest(unittest.TestCase):
    def test_exact_and_explicit_bounds_are_not_inferred(self):
        for bound in ("exact", "lower", "upper"):
            fields = native_fields()
            fields.update(bound=bound, completed_bound=bound)
            row = producer.classify(native_text(fields), A, 2, 24)
            self.assertEqual(row["status"], "ok")
            self.assertEqual(row["observation"]["score_bound"], bound)
        del fields["bound"]
        self.assertEqual(producer.classify(native_text(fields), A, 2, 24)["status"], "incomplete")
        self.assertEqual(producer.classify("info depth 2 score cp 1 pv 5i5h\nbestmove 5i5h\n", A, 2, 24)["status"], "incomplete")

    def test_no_timeout_salvage_unknown_bound_or_partial_depth(self):
        for change in ({"aborted": "true"}, {"abort_reason": "time"}, {"depth": "1"},
                       {"bound": "unknown"}, {"completed_bound": "lower"}, {"completed_iteration_valid": "false"}):
            with self.subTest(change=change):
                row = producer.classify(native_text(native_fields() | change), A, 2, 24)
                self.assertEqual(row["status"], "incomplete")
                self.assertIsNone(row["observation"])

    def test_native_fv_mismatch_and_illegal_pv_are_errors(self):
        for change in ({"fv_scale": "16"}, {"pv_legal": "false"}, {"pv_replay_preserves_input": "false"},
                       {"teacher_search": "false"}, {"history_replayed": "true"}, {"pv_usi": "5i4h"}):
            with self.subTest(change=change), self.assertRaises(producer.ProtocolError):
                producer.classify(native_text(native_fields() | change), A, 2, 24)

    def test_mate_sentinels_never_become_cp_training_labels(self):
        for score, kind, value in ((898999, "cp", 898999), (899000, "mate", 1000),
                                   (899997, "mate", 3), (-899997, "mate", -3)):
            row = producer.classify(native_text(native_fields() | {"score_cp": str(score)}), A, 2, 24)
            self.assertEqual(row["observation"]["score"], {"kind": kind, "value": value})
        for change in ({"score_cp": "900000"}, {"bestmove": "resign"}):
            self.assertEqual(producer.classify(native_text(native_fields() | change), A, 2, 24)["status"], "terminal")

    def test_duplicate_record_or_field_and_invalid_depth_are_errors(self):
        text = native_text(native_fields())
        for payload in (text + text, text.rstrip() + "\tbound=exact\n", native_text(native_fields() | {"depth": "3"})):
            with self.subTest(payload=payload), self.assertRaises(producer.ProtocolError):
                producer.classify(payload, A, 2, 24)

    def test_command_keeps_sfen_one_argument_and_sets_only_registered_controls(self):
        command = producer.native_command(Path("/private/mock binary"), Path("/private/owned weights"), A, 4, 24)
        self.assertEqual(command, ["/private/mock binary", "--max-depth", "4", "--teacher-search", "--fv-scale", "24", "--sfen", A,
                                   "--weights", "/private/owned weights", "--nnue-output", "absolute"])


class MockProcessTest(unittest.TestCase):
    def setUp(self):
        self.proc = MagicMock()
        self.proc.stdout.fileno.return_value = 42
        self.proc.poll.return_value = 0
        self.proc.wait.return_value = 0
        self.proc.pid = 999999
        self.request = {"scales": {"fv_scale": 24}, "limits": {"cleanup_seconds": .1}}

    def run_probe(self, chunks, *, selected=True):
        with patch.object(producer.subprocess, "Popen", return_value=self.proc) as popen, \
                patch.object(producer.select, "select", return_value=([self.proc.stdout] if selected else [], [], [])), \
                patch.object(producer.os, "read", side_effect=chunks), patch.object(producer.os, "killpg") as kill:
            row = producer.probe(Path("/private/mock"), None, self.request, A, 2, time.monotonic()+5, Path("/private/tmp"))
        self.proc.stdout.close.assert_called_once()
        return row, popen, kill

    def test_native_success_uses_clean_env_and_no_shell(self):
        row, popen, kill = self.run_probe([native_text(native_fields()).encode(), b""])
        self.assertEqual(row["status"], "ok")
        self.assertTrue(row["termination"]["reaped"])
        self.assertEqual(popen.call_args.kwargs["env"], producer.ENVIRONMENT)
        self.assertTrue(popen.call_args.kwargs["start_new_session"])
        self.assertNotIn("shell", popen.call_args.kwargs)
        kill.assert_not_called()

    def test_timeout_kills_and_reaps_without_salvage(self):
        self.proc.poll.return_value = None
        self.proc.wait.return_value = -9
        row, _, kill = self.run_probe([], selected=False)
        self.assertEqual(row["status"], "timeout")
        self.assertIsNone(row["observation"])
        kill.assert_called_once_with(self.proc.pid, producer.signal.SIGKILL)
        self.assertTrue(row["termination"]["reaped"])

    def test_interruption_is_retained_and_reaped(self):
        self.proc.poll.return_value = None
        row, _, kill = self.run_probe([KeyboardInterrupt()])
        self.assertTrue(row["interrupted"])
        self.assertIsNone(row["observation"])
        kill.assert_called_once()

    def test_output_cap_crash_and_malformed_transcript_fail_closed(self):
        row, _, _ = self.run_probe([b"x"*(producer.MAX_WIRE+1)])
        self.assertEqual(row["status"], "engine_error")
        self.proc.stdout.close.reset_mock()
        self.proc.wait.return_value = 1
        row, _, _ = self.run_probe([native_text(native_fields()).encode(), b""])
        self.assertEqual(row["status"], "engine_error")
        self.assertIsNone(row["observation"])

    def test_unreaped_process_is_never_reported_clean(self):
        self.proc.poll.return_value = None
        self.proc.wait.side_effect = subprocess.TimeoutExpired("mock", .1)
        row, _, kill = self.run_probe([], selected=False)
        self.assertEqual(row["status"], "engine_error")
        self.assertFalse(row["termination"]["reaped"])
        kill.assert_called_once()


class AttemptTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.document = request_fixture(self.root)
        self.request = self.root / "request.json"
        self.output = self.root / "attempt-a1"
        self.addCleanup(patch.stopall)
        patch.object(producer.shutil, "disk_usage", return_value=type("Usage", (), {"free": 100*1024**3})()).start()

    def write(self):
        self.request.write_text(json.dumps(self.document))

    def run_attempt(self, runner=mock_runner):
        self.write()
        return producer.produce(self.request, self.output, runner=runner)

    def rows(self, profile):
        return [json.loads(line) for line in (self.output / (profile + ".jsonl")).read_text().splitlines()]

    def test_complete_mock_producer_roundtrips_through_preflight(self):
        result = self.run_attempt()
        self.assertEqual(result["status"], "complete_observations")
        self.assertEqual(result["counts"]["deep"], {"ok": 3})
        report = preflight.preflight(self.output / "preflight-inputs.json")
        self.assertEqual(report["comparisons"]["train"]["pair_coverage"], 1)
        self.assertFalse(report["training_permitted"])
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o700)
        self.assertFalse((self.output / "completion-reserve.bin").exists())

    def test_absent_proof_stays_incomplete_for_every_position(self):
        def missing(*args):
            return {"status": "incomplete", "observation": None, "reason": "native_bound_completion_proof_missing"}
        result = self.run_attempt(missing)
        self.assertEqual(result["status"], "complete_observations")
        report = preflight.preflight(self.output / "preflight-inputs.json")
        self.assertEqual(report["comparisons"]["train"]["pair_coverage"], 0)
        self.assertEqual(report["comparisons"]["train"]["counts"]["unique_positions"], 2)

    def test_failed_process_stops_and_fills_all_unstarted_rows(self):
        runner = Mock(return_value={"status": "timeout", "observation": None, "reason": "mock timeout"})
        result = self.run_attempt(runner)
        runner.assert_called_once()
        self.assertEqual(result["status"], "failed")
        rows = self.rows("shallow") + self.rows("deep")
        self.assertEqual(len(rows), 6)
        self.assertEqual(sum(row["status"] == "incomplete" for row in rows), 5)
        preflight.preflight(self.output / "preflight-inputs.json")

    def test_interrupt_preserves_current_failure_and_all_unstarted_rows(self):
        runner = Mock(return_value={"status": "incomplete", "observation": None, "reason": "KeyboardInterrupt", "interrupted": True})
        result = self.run_attempt(runner)
        self.assertEqual(result["status"], "interrupted")
        self.assertEqual(len(self.rows("shallow") + self.rows("deep")), 6)
        self.assertTrue(self.rows("shallow")[0]["interrupted"])

    def test_capacity_and_unknown_capacity_refuse_before_any_process(self):
        for free in (0, "unknown"):
            with self.subTest(free=free):
                self.output = self.root / ("attempt-" + str(free))
                runner = Mock()
                with patch.object(producer.shutil, "disk_usage", return_value=type("Usage", (), {"free": free})()):
                    result = self.run_attempt(runner)
                runner.assert_not_called()
                self.assertEqual(result["status"], "failed")
                self.assertEqual(len(self.rows("shallow") + self.rows("deep")), 6)

    def test_wall_admission_and_mid_attempt_capacity_loss_stop_launching(self):
        self.document["limits"]["max_wall_seconds"] = .1
        runner = Mock()
        result = self.run_attempt(runner)
        runner.assert_not_called()
        self.assertIn("wall budget", result["reason"])
        self.output = self.root / "attempt-a2"
        self.document["limits"]["max_wall_seconds"] = 10
        original = producer.check_capacity
        count = 0
        def capacity(*args):
            nonlocal count
            count += 1
            if count == 3:
                raise producer.CapacityError("synthetic capacity loss")
            return original(*args)
        runner = Mock(side_effect=mock_runner)
        with patch.object(producer, "check_capacity", side_effect=capacity):
            result = self.run_attempt(runner)
        self.assertEqual(result["status"], "failed")
        runner.assert_called_once()

    def test_retries_need_new_directory_and_never_import_old_observations(self):
        self.run_attempt()
        before = (self.output / "receipt.json").read_bytes()
        with self.assertRaises(FileExistsError):
            self.run_attempt()
        self.assertEqual((self.output / "receipt.json").read_bytes(), before)
        prior = self.output / "receipt.json"
        self.document["attempt"]["previous_receipt"] = {"path": str(prior), "sha256": producer.file_sha256(prior)}
        self.output = self.root / "attempt-a2"
        runner = Mock(side_effect=mock_runner)
        self.run_attempt(runner)
        self.assertEqual(runner.call_count, 6)
        self.document["profiles"]["shallow"]["observations"] = {"path": "old.jsonl", "sha256": "1"*64}
        self.output = self.root / "attempt-a3"
        with self.assertRaisesRegex(ValueError, "cannot import"):
            self.run_attempt()

    def test_input_changes_invalidate_all_outputs(self):
        def changing(*args):
            with (self.root / "train.tsv").open("a") as stream:
                stream.write("\n")
            return mock_runner(*args)
        result = self.run_attempt(changing)
        self.assertEqual(result["status"], "failed")
        self.assertIn("mutated", result["reason"])
        self.assertFalse((self.output / "preflight-inputs.json").exists())

    def test_wrong_teacher_or_game_split_is_rejected_before_search(self):
        for change in ("binary", "game"):
            self.output = self.root / ("attempt-" + change)
            saved = copy.deepcopy(self.document)
            if change == "binary":
                self.document["provenance"]["teacher"]["binary_sha256"] = "0"*64
            else:
                path = self.root / "validation.tsv"
                path.write_text(tsv(C, 0, "synthetic-train", -1))
                self.document["validation"]["sha256"] = producer.file_sha256(path)
            runner = Mock()
            result = self.run_attempt(runner)
            runner.assert_not_called()
            self.assertEqual(result["status"], "failed")
            self.document = saved

    def test_owned_halfkp_must_be_a_valid_bound_export_before_launch(self):
        weights = self.root / "invalid-synthetic-weights.bin"
        weights.write_bytes(b"invalid format; no model")
        self.document["weights"] = {"path": weights.name, "sha256": producer.file_sha256(weights)}
        self.document["provenance"]["teacher"].update(evaluator="owned-halfkp", weights_sha256=producer.file_sha256(weights))
        runner = Mock()
        result = self.run_attempt(runner)
        self.assertEqual(result["status"], "failed")
        runner.assert_not_called()

    def test_write_failure_never_publishes_usable_preflight_manifest(self):
        with patch.object(producer, "durable_row", side_effect=OSError("synthetic ENOSPC")):
            result = self.run_attempt()
        self.assertEqual(result["status"], "failed")
        self.assertFalse((self.output / "preflight-inputs.json").exists())

    def test_resource_limits_and_backend_contract_are_explicit(self):
        for section, key, value in (("limits", "per_position_seconds", float("nan")),
                                    ("limits", "max_positions", 129), ("scales", "fv_scale", 0)):
            document = copy.deepcopy(self.document)
            document[section][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                producer.validate_request(document)
        self.document["provenance"]["teacher"]["search"]["hash_mb"] = 32
        with self.assertRaises(ValueError):
            producer.validate_request(self.document)

    def test_existing_teacher_lock_prevents_duplicate_launches_and_is_preserved(self):
        lock = self.root / (".teacher-" + self.document["binary"]["sha256"] + ".lock")
        lock.write_text("another active or unverified previous attempt\n")
        runner = Mock()
        result = self.run_attempt(runner)
        self.assertEqual(result["status"], "failed")
        runner.assert_not_called()
        self.assertEqual(lock.read_text(), "another active or unverified previous attempt\n")

    def test_unreaped_teacher_retains_lock_for_a_separate_process_audit(self):
        runner = Mock(return_value={"status": "engine_error", "observation": None,
                                    "termination": {"pid": 999999, "reaped": False}})
        result = self.run_attempt(runner)
        self.assertEqual(result["status"], "failed")
        self.assertTrue(Path(result["retained_process_lock"]).exists())
        runner.assert_called_once()


if __name__ == "__main__":
    unittest.main()
