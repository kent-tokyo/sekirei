#!/usr/bin/env python3
"""Unit tests for explicit evaluator and thread-scaling options."""

import argparse
import io
import subprocess
import sys
import unittest
import hashlib
from tempfile import TemporaryDirectory
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from nps_threads import (  # noqa: E402
    ELAPSED_SCOPE,
    bind_yaneuraou_evaluator,
    reject_evaluator_overrides,
    Usi,
    engine_options,
    format_optional_ratio,
    go_command,
    positive_u64,
    main as nps_main,
    measure,
    parse_bestmove,
    parse_cooperative_root,
    parse_cooperative_root_prestart,
    parse_cooperative_root_prestart_cap,
    COOPERATIVE_ROOT_PRESTART_CAP_KEYS,
    parse_cooperative_root_prestart_diag,
    COOPERATIVE_ROOT_PRESTART_KEYS,
    COOPERATIVE_ROOT_PRESTART_DIAG_KEYS,
    parse_cooperative_root_wait,
    parse_cooperative_root_latency,
    parse_cooperative_root_policy,
    parse_cooperative_root_helper_lmr_policy,
    parse_cooperative_root_helper_lmr,
    COOPERATIVE_ROOT_HELPER_LMR_KEYS,
    parse_cooperative_root_outcomes,
    COOPERATIVE_ROOT_OUTCOME_KEYS,
    COOPERATIVE_ROOT_LATENCY_KEYS,
    parse_lazy_smp,
    parse_lazy_smp_overlap,
    parse_lazy_smp_root_hint,
    parse_lazy_smp_tt,
    require_lazy_diagnostic_record,
    summarize_lazy_overlap,
    summarize_lazy_root_hints,
    summarize_lazy_root_work,
    summarize_lazy_tt,
    summarize_selected_tt_reuse,
)


class ThreadOptionsTest(unittest.TestCase):
    @staticmethod
    def args(*, yaneuraou: bool, evalfile: str | None) -> argparse.Namespace:
        return argparse.Namespace(
            yaneuraou=yaneuraou,
            evalfile=evalfile,
            fv_scale=24,
            hash=64,
            option=[],
            lazy_diagnostics=False,
        )

    def test_material_only_sekirei_omits_evalfile(self):
        options = engine_options(self.args(yaneuraou=False, evalfile=None), 4)

        self.assertIn("Threads=4", options)
        self.assertFalse(any(option.startswith("EvalFile=") for option in options))

    def test_explicit_sekirei_evalfile_is_preserved(self):
        options = engine_options(self.args(yaneuraou=False, evalfile="nn.bin"), 2)

        self.assertIn(f"EvalFile={Path('nn.bin').resolve()}", options)

    @staticmethod
    def measurement_args(*, yaneuraou, evalfile, engine="fake-engine"):
        args = ThreadOptionsTest.args(yaneuraou=yaneuraou, evalfile=evalfile)
        args.engine = str(engine)
        args.repeats = 1
        args.movetime = 300
        args.depth = 3
        args.require_lazy_diagnostics = False
        args.require_root_hint_diagnostics = False
        args.provenance = {"evaluator": {"kind": "halfkp" if evalfile else "material-only"}}
        return args

    @staticmethod
    def fake_response(token):
        if token == "bestmove":
            return ["info depth 3 score cp 12 nodes 100 nps 1000 pv 7g7f", "bestmove 7g7f"]
        return [token]

    def test_yaneuraou_options_preserve_requested_nn_symlink_directory(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            storage = root / "storage"
            storage.mkdir()
            actual = storage / "weights.bin"
            actual.write_bytes(b"synthetic evaluator")
            selected = root / "native" / "nn.bin"
            selected.parent.mkdir()
            selected.symlink_to(actual)
            options = engine_options(self.args(yaneuraou=True, evalfile=str(selected)), 2)
            self.assertIn(f"EvalDir={selected.parent}", options)
            self.assertNotIn(f"EvalDir={storage}", options)
            engine = root / "engine"
            engine.write_bytes(b"synthetic native binary")
            args = self.measurement_args(yaneuraou=True, evalfile=str(selected), engine=engine)
            with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
                usi.return_value.wait_for.side_effect = self.fake_response
                rows = measure(args, 2, ["startpos"])
                usi.return_value.quit.assert_called_once_with()
            self.assertIn(f"EvalDir={selected.parent}", rows[0]["provenance"]["options"])

    def test_yaneuraou_wrong_native_nn_rejects_before_engine_start(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            selected = root / "selected.bin"
            selected.write_bytes(b"requested synthetic evaluator")
            (root / "nn.bin").write_bytes(b"different native evaluator")
            engine = root / "engine"
            engine.write_bytes(b"synthetic binary")
            args = self.measurement_args(yaneuraou=True, evalfile=str(selected), engine=engine)
            with patch("nps_threads.Usi") as usi, self.assertRaises(ValueError):
                measure(args, 1, ["startpos"])
            usi.assert_not_called()

    def test_all_nps_lanes_reject_evaluator_overrides_and_injection_before_start(self):
        for yaneuraou, evalfile in ((False, None), (False, "weights.bin"), (True, "weights.bin")):
            for option in ("EvalFile=other.bin", " EvalDir =other", "fV_sCaLe=24",
                           "nnueoutput=relative", "Hash=1\nquit", "Hash=1\rquit", "Hash=1\x00", "malformed", "FV_SCALE value 16=ignored",
                           "FV_SCALE ignored 16=ignored", "EvalDir\tvalue /other=ignored"):
                args = self.measurement_args(yaneuraou=yaneuraou, evalfile=evalfile)
                args.option = [option]
                with self.subTest(yaneuraou=yaneuraou, evalfile=evalfile, option=option), patch("nps_threads.Usi") as usi, self.assertRaises(ValueError):
                    measure(args, 1, ["startpos"])
                usi.assert_not_called()

    def test_absolute_output_assertion_is_accepted_without_allowing_other_values(self):
        reject_evaluator_overrides(["NnueOutput=absolute", "Hash=64"])
        for option in ("NnueOutput=relative", "NnueOutput=Absolute", "NnueOutput= absolute", "NnueOutput=absolute "):
            with self.subTest(option=option), self.assertRaises(ValueError):
                reject_evaluator_overrides([option])

    def test_yaneuraou_matching_binding_is_recorded_for_synthetic_search(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            selected = root / "selected.bin"
            contents = b"synthetic shared evaluator"
            selected.write_bytes(contents)
            (root / "nn.bin").write_bytes(contents)
            engine = root / "engine"
            binary = b"synthetic native binary"
            engine.write_bytes(binary)
            args = self.measurement_args(yaneuraou=True, evalfile=str(selected), engine=engine)
            args.provenance.update(
                evaluator={"path": str(selected.resolve()), "sha256": hashlib.sha256(contents).hexdigest()},
                engine=str(engine.resolve()), engine_sha256=hashlib.sha256(binary).hexdigest(),
            )
            expected = bind_yaneuraou_evaluator(str(selected), 24)
            with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
                usi.return_value.wait_for.side_effect = self.fake_response
                rows = measure(args, 2, ["startpos"])
                usi.return_value.quit.assert_called_once_with()
            provenance = rows[0]["provenance"]
            self.assertEqual(provenance["yaneuraou_evaluator_binding"], expected)
            self.assertEqual(provenance["yaneuraou_engine_fingerprint"]["sha256"], hashlib.sha256(binary).hexdigest())
            self.assertEqual(provenance["yaneuraou_engine_fingerprint"]["size_bytes"], len(binary))
            self.assertIn(f"EvalDir={root}", provenance["options"])

    def test_yaneuraou_stale_cli_snapshot_rejects_before_engine_start(self):
        for stale in ("evaluator_hash", "evaluator_path", "engine_hash", "engine_path"):
            with self.subTest(stale=stale), TemporaryDirectory() as tmp:
                root = Path(tmp)
                native = root / "nn.bin"
                native.write_bytes(b"synthetic shared evaluator")
                engine = root / "engine"
                engine.write_bytes(b"synthetic native binary")
                other = root / "other"
                other.write_bytes(b"synthetic shared evaluator" if stale == "evaluator_path" else b"synthetic native binary")
                args = self.measurement_args(yaneuraou=True, evalfile=str(native), engine=engine)
                args.provenance.update(
                    evaluator={"path": str(native.resolve()), "sha256": hashlib.sha256(native.read_bytes()).hexdigest()},
                    engine=str(engine.resolve()), engine_sha256=hashlib.sha256(engine.read_bytes()).hexdigest(),
                )
                if stale == "evaluator_hash":
                    native.write_bytes(b"changed after CLI snapshot")
                elif stale == "evaluator_path":
                    args.provenance["evaluator"]["path"] = str(other.resolve())
                elif stale == "engine_hash":
                    engine.write_bytes(b"changed after CLI snapshot")
                else:
                    args.provenance["engine"] = str(other.resolve())
                with patch("nps_threads.Usi") as usi, self.assertRaisesRegex(ValueError, "declared"):
                    measure(args, 1, ["startpos"])
                usi.assert_not_called()

    def test_yaneuraou_input_mutation_discards_rows_and_quits(self):
        for mutated in ("requested", "native", "engine"):
            with self.subTest(mutated=mutated), TemporaryDirectory() as tmp:
                root = Path(tmp)
                selected = root / "selected.bin"
                native = root / "nn.bin"
                selected.write_bytes(b"synthetic shared evaluator")
                native.write_bytes(b"synthetic shared evaluator")
                engine = root / "engine"
                engine.write_bytes(b"synthetic native binary")
                args = self.measurement_args(yaneuraou=True, evalfile=str(selected), engine=engine)
                target = {"requested": selected, "native": native, "engine": engine}[mutated]
                def response(token):
                    if token == "bestmove":
                        target.write_bytes(b"modified during measurement")
                    return self.fake_response(token)
                with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
                    usi.return_value.wait_for.side_effect = response
                    with self.assertRaises(ValueError):
                        measure(args, 1, ["startpos"])
                    usi.return_value.quit.assert_called_once_with()

    def test_yaneuraou_shutdown_mutation_is_checked_before_accepting_rows(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            native = root / "nn.bin"
            native.write_bytes(b"synthetic shared evaluator")
            engine = root / "engine"
            engine.write_bytes(b"synthetic native binary")
            args = self.measurement_args(yaneuraou=True, evalfile=str(native), engine=engine)
            with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
                usi.return_value.wait_for.side_effect = self.fake_response
                usi.return_value.quit.side_effect = lambda: engine.write_bytes(b"changed at shutdown")
                with self.assertRaises(ValueError):
                    measure(args, 1, ["startpos"])
                usi.return_value.quit.assert_called_once_with()

    def test_ordinary_sekirei_measurement_has_no_new_file_binding_reads(self):
        for evalfile in (None, "does-not-exist.bin"):
            args = self.measurement_args(yaneuraou=False, evalfile=evalfile)
            args.option = ["NnueOutput=absolute"]
            with self.subTest(evalfile=evalfile), patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"), patch("nps_threads.file_fingerprint") as fingerprint, patch("nps_threads.bind_yaneuraou_evaluator") as binding:
                usi.return_value.wait_for.side_effect = self.fake_response
                row = measure(args, 1, ["startpos"])[0]
                fingerprint.assert_not_called()
                binding.assert_not_called()
                self.assertNotIn("yaneuraou_evaluator_binding", row["provenance"])

    def test_relative_engine_path_is_resolved_before_changing_directory(self):
        with patch("nps_threads.subprocess.Popen") as popen:
            Usi("target/release/sekirei", {})

        binary = Path("target/release/sekirei").resolve()
        self.assertEqual(popen.call_args.args[0], [str(binary)])
        self.assertEqual(popen.call_args.kwargs["cwd"], str(binary.parent))

    def test_quit_reaps_an_engine_that_does_not_exit_in_time(self):
        with patch("nps_threads.subprocess.Popen") as popen:
            process = popen.return_value
            process.wait.side_effect = [
                subprocess.TimeoutExpired("sekirei", 5),
                0,
            ]
            engine = Usi("target/release/sekirei", {})
            engine.quit()

        process.kill.assert_called_once_with()
        self.assertEqual(process.wait.call_count, 2)

    def test_wait_for_collects_response_and_detects_early_exit(self):
        with patch("nps_threads.subprocess.Popen") as popen:
            popen.return_value.stdout = io.StringIO("info depth 1\nbestmove 7g7f\n")
            engine = Usi("engine", {}, timeout=1)
            self.assertEqual(engine.wait_for("bestmove"), ["info depth 1", "bestmove 7g7f"])
            with self.assertRaisesRegex(RuntimeError, "exited"):
                engine.wait_for("readyok")

    def test_position_ready_barrier_excludes_pending_reset_on_every_repeat(self):
        for reported_nps in (98765, 0):
            with self.subTest(reported_nps=reported_nps):
                args = self.args(yaneuraou=False, evalfile=None)
                args.engine = "fake-engine"
                args.repeats = 2
                args.movetime = 1000
                args.depth = None
                args.require_lazy_diagnostics = False
                args.require_root_hint_diagnostics = False
                args.provenance = {"counter_overhead_included": False}
                clock = 0.0
                pending_reset = 0.0
                events = []

                def send(line):
                    nonlocal pending_reset
                    events.append(("send", line))
                    if line == "usinewgame":
                        pending_reset += 0.5

                def wait_for(token):
                    nonlocal clock, pending_reset
                    events.append(("wait", token))
                    # A queued reset is processed before either readyok or
                    # bestmove; omitting the barrier would time an extra 500ms.
                    clock += pending_reset
                    pending_reset = 0.0
                    if token == "bestmove":
                        clock += 0.125
                        return [f"info depth 4 score cp 0 nodes 900 nps {reported_nps} pv 7g7f",
                                "bestmove 7g7f"]
                    return [token]

                def now():
                    events.append(("clock", clock))
                    return clock

                with patch("nps_threads.Usi") as usi, patch("nps_threads.time.perf_counter", side_effect=now), patch("nps_threads.sys.stderr"):
                    usi.return_value.send.side_effect = send
                    usi.return_value.wait_for.side_effect = wait_for
                    rows = measure(args, 4, ["startpos", "startpos moves 7g7f"])
                    usi.return_value.quit.assert_called_once_with()

                self.assertEqual(len(rows), 4)
                self.assertEqual([row["elapsed_ms"] for row in rows], [125] * 4)
                self.assertEqual([row["nps"] for row in rows],
                                 [reported_nps or 7200] * 4)
                self.assertTrue(all(row["provenance"]["elapsed_scope"] == ELAPSED_SCOPE
                                    for row in rows))
                for index, event in enumerate(events):
                    if event == ("send", "go movetime 1000"):
                        self.assertEqual(events[index - 1][0], "clock")
                        self.assertEqual(events[index - 2], ("wait", "readyok"))
                        self.assertEqual(events[index - 3], ("send", "isready"))
                        self.assertTrue(events[index - 4][1].startswith("position "))
                self.assertEqual(events.count(("wait", "readyok")), 5)

    def test_lazy_smp_diagnostics_are_parsed(self):
        diagnostics = parse_lazy_smp(
            [
                "info string lazy_smp selected w2 node_share_permille 268 "
                "move_agreement 3/4 discarded_root_nodes 33 "
                "divergent_discarded_nodes 10 stop_lag_ms 2 "
                "selected_tt_local_hits 7 selected_tt_foreign_hits 5 "
                "selected_tt_unknown_hits 2 "
                "w0:d20:n10:s1:t300:x1:aexternal:p30:h20:r10:l8:f3:u1,"
                "w1:d20:n11:s2:t301:x1:aexternal:p31:h21:r11:l9:f4:u1,"
                "w2*:d21:n12:s3:t302:x0:acompleted:p32:h22:r12:l7:f5:u2,"
                "w3:d20:n12:s3:t301:x1:aexternal:p33:h23:r13:l9:f6:u1"
            ]
        )

        self.assertEqual(
            diagnostics,
            {
                "selected_worker": 2,
                "selected_node_share_permille": 268,
                "move_agreement": 3,
                "workers": 4,
                "discarded_root_nodes": 33,
                "divergent_discarded_nodes": 10,
                "stop_lag_ms": 2,
                "selected_tt_local_hits": 7,
                "selected_tt_foreign_hits": 5,
                "selected_tt_unknown_hits": 2,
                "worker_stats": [
                    {
                        "worker": 0,
                        "selected": False,
                        "depth": 20,
                        "nodes": 10,
                        "score": 1,
                        "elapsed_ms": 300,
                        "aborted": True,
                        "abort_reason": "external",
                        "tt_probes": 30,
                        "tt_hits": 20,
                        "tt_stores": 10,
                        "tt_local_hits": 8,
                        "tt_foreign_hits": 3,
                        "tt_unknown_hits": 1,
                    },
                    {
                        "worker": 1,
                        "selected": False,
                        "depth": 20,
                        "nodes": 11,
                        "score": 2,
                        "elapsed_ms": 301,
                        "aborted": True,
                        "abort_reason": "external",
                        "tt_probes": 31,
                        "tt_hits": 21,
                        "tt_stores": 11,
                        "tt_local_hits": 9,
                        "tt_foreign_hits": 4,
                        "tt_unknown_hits": 1,
                    },
                    {
                        "worker": 2,
                        "selected": True,
                        "depth": 21,
                        "nodes": 12,
                        "score": 3,
                        "elapsed_ms": 302,
                        "aborted": False,
                        "abort_reason": "completed",
                        "tt_probes": 32,
                        "tt_hits": 22,
                        "tt_stores": 12,
                        "tt_local_hits": 7,
                        "tt_foreign_hits": 5,
                        "tt_unknown_hits": 2,
                    },
                    {
                        "worker": 3,
                        "selected": False,
                        "depth": 20,
                        "nodes": 12,
                        "score": 3,
                        "elapsed_ms": 301,
                        "aborted": True,
                        "abort_reason": "external",
                        "tt_probes": 33,
                        "tt_hits": 23,
                        "tt_stores": 13,
                        "tt_local_hits": 9,
                        "tt_foreign_hits": 6,
                        "tt_unknown_hits": 1,
                    },
                ],
            },
        )

    def test_malformed_lazy_smp_diagnostics_are_ignored(self):
        self.assertIsNone(parse_lazy_smp(["info string lazy_smp selected nope"]))

    def test_lazy_smp_tt_diagnostics_are_parsed(self):
        diagnostics = parse_lazy_smp_tt(
            [
                "info string lazy_smp_tt probes 100 hits 61 attempted 40 "
                "committed 35 same_hash 20 equal_depth 4 shallower_rejected 3 "
                "collision_overwrites 11"
            ]
        )

        self.assertEqual(
            diagnostics,
            {
                "probes": 100,
                "hits": 61,
                "attempted": 40,
                "committed": 35,
                "same_hash": 20,
                "equal_depth": 4,
                "shallower_rejected": 3,
                "collision_overwrites": 11,
            },
        )

    def test_lazy_smp_overlap_diagnostics_are_parsed(self):
        diagnostics = parse_lazy_smp_overlap(
            [
                "info string lazy_smp_overlap sample_rate 256 visits 100 "
                "unique 40 within_worker_revisits 35 cross_worker_visits 25 "
                "shared_fingerprints 12 dropped 0 tt_sampled_stores 50 "
                "tt_sampled_hits 30 tt_local_hits 18 tt_foreign_hits 9 "
                "tt_unknown_hits 3 tt_dropped_stores 1"
            ]
        )

        self.assertEqual(
            diagnostics,
            {
                "sample_rate": 256,
                "visits": 100,
                "unique": 40,
                "within_worker_revisits": 35,
                "cross_worker_visits": 25,
                "shared_fingerprints": 12,
                "dropped": 0,
                "tt_sampled_stores": 50,
                "tt_sampled_hits": 30,
                "tt_local_hits": 18,
                "tt_foreign_hits": 9,
                "tt_unknown_hits": 3,
                "tt_dropped_stores": 1,
            },
        )

    def test_root_hint_diagnostics_are_parsed(self):
        self.assertEqual(
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications 7 "
                    "eligible_reads 5 applied 3"
                ],
                required=True,
            ),
            {"publications": 7, "eligible_reads": 5, "applied": 3},
        )

    def test_required_root_hint_diagnostic_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            parse_lazy_smp_root_hint([], required=True)
        with self.assertRaisesRegex(ValueError, "expected one"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications 1 eligible_reads 1 applied 1",
                    "info string lazy_smp_root_hint publications 1 eligible_reads 1 applied 1",
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "applications exceed"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications 1 "
                    "eligible_reads 1 applied 2"
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "without a publication"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications 0 "
                    "eligible_reads 1 applied 0"
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "malformed"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint eligible_reads 1 "
                    "publications 1 applied 0"
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "unsigned decimal"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications +1 "
                    "eligible_reads 1 applied 0"
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "exceeds u64"):
            parse_lazy_smp_root_hint(
                [
                    f"info string lazy_smp_root_hint publications {1 << 64} "
                    "eligible_reads 0 applied 0"
                ],
                required=True,
            )

    def test_bestmove_parser_uses_terminal_response(self):
        self.assertEqual(
            parse_bestmove(["info depth 4 pv 7g7f", "bestmove 7g7f ponder 3c3d"]),
            "7g7f",
        )

    def test_lazy_diagnostics_option_is_explicit(self):
        args = self.args(yaneuraou=False, evalfile=None)
        args.lazy_diagnostics = True

        self.assertIn("LazyDiagnostics=true", engine_options(args, 4))

    def test_required_lazy_diagnostics_enable_the_engine_counters(self):
        args = self.args(yaneuraou=False, evalfile=None)
        args.require_lazy_diagnostics = True

        self.assertIn("LazyDiagnostics=true", engine_options(args, 4))

    def test_required_lazy_diagnostics_fail_on_partial_records(self):
        complete = {
            "worker_stats": [{"worker": 0, "selected": True}],
            "selected_tt_local_hits": 1,
            "selected_tt_foreign_hits": 2,
            "selected_tt_unknown_hits": 0,
        }
        require_lazy_diagnostic_record(complete, {"probes": 3}, {"visits": 4})

        with self.assertRaisesRegex(ValueError, "selected_tt_foreign_hits"):
            require_lazy_diagnostic_record(
                {key: value for key, value in complete.items() if key != "selected_tt_foreign_hits"},
                {"probes": 3},
                {"visits": 4},
            )
        with self.assertRaisesRegex(ValueError, "lazy_smp_overlap"):
            require_lazy_diagnostic_record(complete, {"probes": 3}, None)

    def test_cooperative_wait_evidence_is_optional_and_bounded(self):
        root = {"main_confirmations": 3, "confirmation_nodes": 100, "confirmed_alpha_raises": 2}
        line = ("info string cooperative_root_wait enabled 1 waiting_confirmations 2 "
                "waiting_confirmation_nodes 80 waiting_alpha_raises 1 notification_wait_us 50")
        self.assertIsNone(parse_cooperative_root_wait([], cooperative_root=None))
        parsed = parse_cooperative_root_wait([line], cooperative_root=root)
        self.assertEqual(parsed, {"enabled": 1, "waiting_confirmations": 2,
                                 "waiting_confirmation_nodes": 80,
                                 "waiting_alpha_raises": 1, "notification_wait_us": 50})
        for bad in ([line, line], [line + " extra 0"], [line.replace("enabled 1", "enabled 0")],
                    [line.replace("enabled 1", "enabled 2")],
                    [line.replace("waiting_confirmations 2", "waiting_confirmations 4")],
                    [line.replace("waiting_confirmation_nodes 80", "waiting_confirmation_nodes 101")],
                    [line.replace("waiting_alpha_raises 1", "waiting_alpha_raises 3")],
                    [line.replace("waiting_confirmations 2", "waiting_confirmations 0")],
                    [line.replace("notification_wait_us 50", "notification_wait_us -1")],
                    [line.replace("notification_wait_us 50", "notification_wait_us ５０")],
                    [line.replace("notification_wait_us 50", f"notification_wait_us {1 << 64}")]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_cooperative_root_wait(bad, cooperative_root=root)
        with self.assertRaisesRegex(ValueError, "requires enabled split"):
            parse_cooperative_root_wait([line], cooperative_root=None)
        with self.assertRaisesRegex(ValueError, "confirmed raises"):
            parse_cooperative_root_wait([line], cooperative_root={**root, "confirmed_alpha_raises": 0})

    def test_cooperative_latency_parses_optional_overlapping_delay_evidence(self):
        root = {"helper_fail_highs": 6, "main_confirmations": 3, "main_nodes": 1000, "confirmed_alpha_raises": 2}
        stats = dict.fromkeys(COOPERATIVE_ROOT_LATENCY_KEYS, 0)
        stats.update(publications=5, scout_publications=2, research_publications=1,
                     confirmation_publications=1, idle_publications=1,
                     beta_scout_publications=1, confirmations=3,
                     confirmation_latency_us_sum=120, confirmation_latency_us_max=70,
                     confirmation_main_nodes_sum=1800, confirmation_main_nodes_max=900,
                     scout_confirmations=2, scout_delay_nodes_sum=1500,
                     beta_scout_confirmations=1, beta_scout_delay_nodes_sum=700,
                     beta_scout_confirmed_cuts=1, unconfirmed_publications=2,
                     rejected_publications=1)
        def line(values=stats):
            return "info string cooperative_root_latency enabled 1 " + " ".join(f"{key} {values[key]}" for key in COOPERATIVE_ROOT_LATENCY_KEYS)
        self.assertIsNone(parse_cooperative_root_latency([], cooperative_root=None))
        parsed = parse_cooperative_root_latency([line()], cooperative_root=root)
        self.assertEqual(parsed, {"enabled": 1, **stats})
        # Candidate delays can overlap: their sum can exceed main's total nodes.
        self.assertGreater(parsed["confirmation_main_nodes_sum"], root["main_nodes"])
        for bad in ([line(), line()], [line() + " extra 0"],
                    [line().replace("enabled 1", "enabled 0")],
                    [line().replace("enabled 1", "enabled 2")],
                    [line().replace("publications 5", "publications -1", 1)],
                    [line().replace("publications 5", "publications ５", 1)],
                    [line().replace("publications 5", f"publications {1 << 64}", 1)]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_cooperative_root_latency(bad, cooperative_root=root)
        for key, value in (("idle_publications", 2), ("rejected_publications", 2),
                           ("confirmations", 4), ("unconfirmed_publications", 3),
                           ("beta_scout_publications", 3), ("scout_confirmations", 3),
                           ("beta_scout_confirmations", 2), ("beta_scout_confirmed_cuts", 2),
                           ("confirmation_latency_us_max", 121), ("confirmation_main_nodes_max", 1801),
                           ("confirmation_main_nodes_max", 1001),
                           ("scout_delay_nodes_sum", 1801), ("beta_scout_delay_nodes_sum", 1501)):
            with self.subTest(key=key), self.assertRaises(ValueError):
                parse_cooperative_root_latency([line({**stats, key: value})], cooperative_root=root)
        with self.assertRaisesRegex(ValueError, "requires enabled split"):
            parse_cooperative_root_latency([line()], cooperative_root=None)
        with self.assertRaisesRegex(ValueError, "cuts exceed"):
            parse_cooperative_root_latency([line()], cooperative_root={**root, "confirmed_alpha_raises": 0})
        with self.assertRaisesRegex(ValueError, "confirmed/unconfirmed"):
            values = {**stats, "confirmations": 6, "unconfirmed_publications": 0}
            parse_cooperative_root_latency([line(values)], cooperative_root={**root, "main_confirmations": 6})

    def test_measure_records_actual_latency_observer_overhead_without_drain(self):
        args = self.args(yaneuraou=False, evalfile=None)
        args.engine = "fake-engine"
        args.repeats = 1
        args.movetime = 300
        args.depth = 3
        args.require_lazy_diagnostics = False
        args.require_root_hint_diagnostics = False
        args.provenance = {"counter_overhead_included": False}
        args.option = ["T_COOP_LATENCY=1"]
        root_line = ("info string cooperative_root generations 1 helper_nodes 0 helper_probes 0 "
                     "helper_fail_highs 0 main_confirmations 0 confirmed_alpha_raises 0 "
                     "main_nodes 100 main_tasks 1 confirmation_nodes 0 helper_completed_nodes 0 "
                     "helper_aborted_nodes 0 helper_aborted_probes 0 spawn_failures 0 helpers_started 1 "
                     "helper_start_delay_us 0 join_wait_us 0 alpha_gap_sum 0 alpha_gap_max 0 "
                     "physical_helper_threads_started 1 physical_helper_threads_joined 1 "
                     "executor_start_delay_us 0 helper_dispatch_delay_us 0")
        latency_line = "info string cooperative_root_latency enabled 1 " + " ".join(f"{key} 0" for key in COOPERATIVE_ROOT_LATENCY_KEYS)
        outputs = [root_line, latency_line, "info depth 3 score cp 0 nodes 100 nps 1000 pv 7g7f", "bestmove 7g7f"]
        with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
            usi.return_value.wait_for.side_effect = lambda token: outputs if token == "bestmove" else [token]
            row = measure(args, 2, ["startpos"])[0]
        self.assertIsNone(row["cooperative_root_wait"])
        self.assertEqual(row["cooperative_root_latency"]["enabled"], 1)
        self.assertTrue(row["provenance"]["cooperative_latency_overhead_included"])
        self.assertTrue(row["provenance"]["counter_overhead_included"])
        self.assertFalse(row["provenance"]["timing_qualified"])

    @staticmethod
    def cooperative_root_line(workers=2):
        started = 2 * (workers - 1)
        values = {
            "generations": 2, "helper_nodes": 30, "helper_probes": 3,
            "helper_fail_highs": 1, "main_confirmations": 1, "confirmed_alpha_raises": 1,
            "main_nodes": 100, "main_tasks": 2, "confirmation_nodes": 20,
            "helper_completed_nodes": 25, "helper_aborted_nodes": 5,
            "helper_aborted_probes": 1, "spawn_failures": 0, "helpers_started": started,
            "helper_start_delay_us": 200, "join_wait_us": 400,
            "alpha_gap_sum": 10, "alpha_gap_max": 10,
            "physical_helper_threads_started": started, "physical_helper_threads_joined": started,
            "executor_start_delay_us": 0, "helper_dispatch_delay_us": 0,
        }
        return "info string cooperative_root " + " ".join(f"{key} {value}" for key, value in values.items())

    def test_cooperative_groups_allow_executor_reuse_without_losing_job_census(self):
        for workers in (4, 8):
            helpers = workers - 1
            jobs = 2 * helpers
            group = f"info string cooperative_root_group workers {workers} helpers {helpers}"
            root = self.cooperative_root_line(workers).replace(
                f"physical_helper_threads_started {jobs} physical_helper_threads_joined {jobs}",
                f"physical_helper_threads_started {helpers} physical_helper_threads_joined {helpers}",
            )
            with self.subTest(workers=workers):
                result = parse_cooperative_root([group, root], workers=workers, total_nodes=130)
                self.assertEqual(result["helpers_started"], jobs)
                self.assertEqual(result["physical_helper_threads_started"], helpers)
                self.assertEqual(result["physical_helper_threads_joined"], helpers)
                self.assertEqual(result["spawn_failures"], 0)
                with self.assertRaises(ValueError):
                    parse_cooperative_root([group, root.replace("helpers_started " + str(jobs),
                                                               "helpers_started " + str(jobs - 1))])
                with self.assertRaises(ValueError):
                    parse_cooperative_root([group, root.replace("physical_helper_threads_joined " + str(helpers),
                                                               "physical_helper_threads_joined " + str(helpers - 1))])

    def test_cooperative_groups_bind_size_and_preserve_legacy_contract(self):
        self.assertIsNone(parse_cooperative_root([], workers=4))
        legacy = parse_cooperative_root([self.cooperative_root_line()], workers=2)
        self.assertNotIn("workers", legacy)
        self.assertNotIn("helpers", legacy)
        with self.assertRaisesRegex(ValueError, "requested threads"):
            parse_cooperative_root([self.cooperative_root_line()], workers=4)
        for workers in (4, 8):
            group = f"info string cooperative_root_group workers {workers} helpers {workers - 1}"
            root = self.cooperative_root_line(workers)
            with self.subTest(workers=workers):
                result = parse_cooperative_root([group, root], workers=workers, total_nodes=130)
                self.assertEqual((result["workers"], result["helpers"]), (workers, workers - 1))
                self.assertEqual(result["helpers_started"], result["generations"] * result["helpers"])
                self.assertEqual(parse_cooperative_root([root, group]), result)
                with self.assertRaisesRegex(ValueError, "requested threads"):
                    parse_cooperative_root([group, root], workers=2)

    def test_cooperative_groups_reject_malformed_or_orphan_evidence(self):
        group = "info string cooperative_root_group workers 4 helpers 3"
        root = self.cooperative_root_line(4)
        malformed = [
            [group, group, root],
            ["info string cooperative_root_group", root],
            [group.replace("workers 4 helpers 3", "helpers 3 workers 4"), root],
            [group + " extra 0", root],
            [group.replace("workers 4", "workers -4"), root],
            [group.replace("workers 4", "workers ４"), root],
            [group.replace("workers 4", f"workers {1 << 32}"), root],
            [group.replace("helpers 3", f"helpers {1 << 32}"), root],
            [group.replace("workers 4 helpers 3", "workers 2 helpers 1"), root],
            [group.replace("workers 4 helpers 3", "workers 16 helpers 15"), root],
            [group.replace("helpers 3", "helpers 2"), root],
            [group],
            [group, root, root],
        ]
        for lines in malformed:
            with self.subTest(lines=lines), self.assertRaises(ValueError):
                parse_cooperative_root(lines)

    def test_cooperative_groups_preserve_node_and_lifecycle_validation(self):
        group = "info string cooperative_root_group workers 4 helpers 3"
        root = self.cooperative_root_line(4)
        failures = root.replace("spawn_failures 0 helpers_started 6", "spawn_failures 2 helpers_started 4")
        failures = failures.replace("physical_helper_threads_started 6 physical_helper_threads_joined 6",
                                    "physical_helper_threads_started 4 physical_helper_threads_joined 4")
        self.assertEqual(parse_cooperative_root([group, failures])["spawn_failures"], 2)
        for old, new in (
            ("helpers_started 6", "helpers_started 2"),
            ("spawn_failures 0", "spawn_failures 1"),
            ("helper_aborted_probes 1", "helper_aborted_probes 7"),
            ("physical_helper_threads_joined 6", "physical_helper_threads_joined 5"),
            ("physical_helper_threads_started 6 physical_helper_threads_joined 6",
             "physical_helper_threads_started 7 physical_helper_threads_joined 7"),
            ("helper_completed_nodes 25", "helper_completed_nodes 26"),
            ("confirmation_nodes 20", "confirmation_nodes 101"),
            ("alpha_gap_max 10", "alpha_gap_max 11"),
        ):
            with self.subTest(new=new), self.assertRaises(ValueError):
                parse_cooperative_root([group, root.replace(old, new)])
        with self.assertRaisesRegex(ValueError, "total nodes"):
            parse_cooperative_root([group, root], total_nodes=129)

    def test_measure_records_cooperative_group_without_optional_observers(self):
        args = self.measurement_args(yaneuraou=False, evalfile=None)
        args.option = ["T_COOP_MULTI=1"]
        args.provenance["counter_overhead_included"] = False
        group = "info string cooperative_root_group workers 4 helpers 3"
        root = self.cooperative_root_line(4)
        outputs = [group, root, "info depth 3 score cp 0 nodes 130 nps 1000 pv 7g7f", "bestmove 7g7f"]
        with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
            usi.return_value.wait_for.side_effect = lambda token: outputs if token == "bestmove" else [token]
            row = measure(args, 4, ["startpos"])[0]
        self.assertEqual(row["cooperative_root"]["workers"], 4)
        self.assertEqual(row["cooperative_root"]["helpers"], 3)
        self.assertIsNone(row["cooperative_root_wait"])
        self.assertIsNone(row["cooperative_root_latency"])
        self.assertFalse(row["provenance"]["counter_overhead_included"])
        with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
            usi.return_value.wait_for.side_effect = lambda token: outputs if token == "bestmove" else [token]
            with self.assertRaisesRegex(ValueError, "requested threads"):
                measure(args, 8, ["startpos"])

    def test_cooperative_priority_policy_is_intrinsic_and_strict(self):
        line = "info string cooperative_root_policy priority 1"
        self.assertIsNone(parse_cooperative_root_policy([], cooperative_root=None))
        self.assertEqual(parse_cooperative_root_policy([line], cooperative_root={}), {"priority": 1})
        for bad in ([line, line], [line + " extra 0"], [line.replace("priority 1", "priority 0")],
                    [line.replace("priority 1", "priority 2")], [line.replace("priority 1", "priority １")]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_cooperative_root_policy(bad, cooperative_root={})
        with self.assertRaisesRegex(ValueError, "requires split"):
            parse_cooperative_root_policy([line], cooperative_root=None)

    def test_cooperative_outcomes_validate_phase_and_priority_subsets(self):
        root = {"confirmed_alpha_raises": 3, "confirmation_nodes": 100}
        latency = dict(confirmations=6, scout_publications=3, research_publications=2,
                       confirmation_publications=1, idle_publications=1,
                       phase_transition_publications=1, scout_confirmations=2,
                       beta_scout_confirmed_cuts=1)
        stats = dict.fromkeys(COOPERATIVE_ROOT_OUTCOME_KEYS, 0)
        stats.update(scout_confirmations=2, research_confirmations=1,
                     confirmation_confirmations=1, idle_confirmations=1,
                     phase_transition_confirmations=1, scout_beta_cuts=1,
                     priority_choices=6, priority_reorders=2, reordered_alpha_raises=1,
                     reordered_beta_cuts=1, reordered_confirmation_nodes=50)
        def line(values=stats):
            return "info string cooperative_root_outcomes enabled 1 " + " ".join(f"{key} {values[key]}" for key in COOPERATIVE_ROOT_OUTCOME_KEYS)
        def parse(lines, *, root=root, latency=latency, policy={"priority": 1}):
            return parse_cooperative_root_outcomes(lines, cooperative_root=root, latency=latency, policy=policy)
        self.assertIsNone(parse([], root=None, latency=None, policy=None))
        self.assertEqual(parse([line()]), {"enabled": 1, **stats})
        for bad in ([line(), line()], [line() + " extra 0"],
                    [line().replace("enabled 1", "enabled 0")],
                    [line().replace("enabled 1", "enabled 2")],
                    [line().replace("priority_choices 6", "priority_choices -1")],
                    [line().replace("priority_choices 6", "priority_choices ６")],
                    [line().replace("priority_choices 6", f"priority_choices {1 << 64}")]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse(bad)
        for key, value in (("idle_confirmations", 2), ("scout_beta_cuts", 3),
                           ("priority_choices", 5), ("priority_reorders", 7),
                           ("reordered_alpha_raises", 3), ("reordered_beta_cuts", 2),
                           ("reordered_confirmation_nodes", 101)):
            with self.subTest(key=key), self.assertRaises(ValueError):
                parse([line({**stats, key: value})])
        with self.assertRaises(ValueError):
            parse([line()], root=None)
        with self.assertRaises(ValueError):
            parse([line()], latency=None)
        with self.assertRaisesRegex(ValueError, "without enabled policy"):
            parse([line()], policy=None)
        with self.assertRaisesRegex(ValueError, "scout counters"):
            parse([line()], latency={**latency, "beta_scout_confirmed_cuts": 2})
        with self.assertRaisesRegex(ValueError, "scout counters"):
            parse([line()], latency={**latency, "scout_confirmations": 1})
        with self.assertRaisesRegex(ValueError, "beta cuts exceed"):
            parse([line()], root={**root, "confirmed_alpha_raises": 0})
        disabled = {**stats, **dict.fromkeys(COOPERATIVE_ROOT_OUTCOME_KEYS[-5:], 0)}
        self.assertEqual(parse([line(disabled)], policy=None)["priority_choices"], 0)

    def test_measure_observes_priority_without_latency_counter_overhead(self):
        args = self.args(yaneuraou=False, evalfile=None)
        args.engine = "fake-engine"
        args.repeats = 1
        args.movetime = 300
        args.depth = 3
        args.require_lazy_diagnostics = False
        args.require_root_hint_diagnostics = False
        args.provenance = {"counter_overhead_included": False, "timing_qualified": True}
        args.option = ["T_COOP_PRIORITY=1", "T_COOP_LATENCY=0"]
        root_line = ("info string cooperative_root generations 1 helper_nodes 0 helper_probes 0 "
                     "helper_fail_highs 0 main_confirmations 0 confirmed_alpha_raises 0 "
                     "main_nodes 100 main_tasks 1 confirmation_nodes 0 helper_completed_nodes 0 "
                     "helper_aborted_nodes 0 helper_aborted_probes 0 spawn_failures 0 helpers_started 1 "
                     "helper_start_delay_us 0 join_wait_us 0 alpha_gap_sum 0 alpha_gap_max 0 "
                     "physical_helper_threads_started 1 physical_helper_threads_joined 1 "
                     "executor_start_delay_us 0 helper_dispatch_delay_us 0")
        outputs = [root_line, "info string cooperative_root_policy priority 1", "info depth 3 score cp 0 nodes 100 nps 1000 pv 7g7f", "bestmove 7g7f"]
        with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
            usi.return_value.wait_for.side_effect = lambda token: outputs if token == "bestmove" else [token]
            row = measure(args, 2, ["startpos"])[0]
        self.assertEqual(row["cooperative_root_policy"], {"priority": 1})
        self.assertIsNone(row["cooperative_root_latency"])
        self.assertIsNone(row["cooperative_root_outcomes"])
        self.assertTrue(row["provenance"]["cooperative_priority_enabled"])
        self.assertFalse(row["provenance"]["counter_overhead_included"])
        self.assertTrue(row["provenance"]["timing_qualified"])

    def test_helper_lmr_policy_is_optional_intrinsic_identity(self):
        line = "info string cooperative_root_helper_lmr_policy enabled 1"
        self.assertIsNone(parse_cooperative_root_helper_lmr_policy([], cooperative_root=None))
        self.assertEqual(parse_cooperative_root_helper_lmr_policy([line], cooperative_root={}), {"enabled": 1})
        for bad in ([line, line], [line + " extra 0"], [line.replace("enabled 1", "enabled 0")],
                    [line.replace("enabled 1", "enabled 2")], [line.replace("enabled 1", "enabled １")]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_cooperative_root_helper_lmr_policy(bad, cooperative_root={})
        with self.assertRaisesRegex(ValueError, "requires split"):
            parse_cooperative_root_helper_lmr_policy([line], cooperative_root=None)

    def test_helper_lmr_counters_preserve_legacy_full_depth_probe_accounting(self):
        stats = dict(reduced_probes=6, reduced_fail_highs=2, full_depth_rechecks=2,
                     full_depth_fail_highs=1, completed_reduced_fail_lows=3)
        def line(values=stats):
            return "info string cooperative_root_helper_lmr " + " ".join(f"{key} {values[key]}" for key in COOPERATIVE_ROOT_HELPER_LMR_KEYS)
        def parse(lines, *, root={"helper_probes": 0, "helper_fail_highs": 0}, policy={"enabled": 1}, latency={}):
            return parse_cooperative_root_helper_lmr(lines, cooperative_root=root, policy=policy, latency=latency)
        self.assertIsNone(parse([], root=None, policy=None, latency=None))
        # Reduced fail lows do not count as legacy full-depth completions, and
        # cancellation can separate rechecks from legacy publication counts.
        self.assertEqual(parse([line()]), stats)
        for bad in ([line(), line()], [line() + " extra 0"],
                    [line().replace("reduced_probes 6", "reduced_probes -1")],
                    [line().replace("reduced_probes 6", "reduced_probes ６")],
                    [line().replace("reduced_probes 6", f"reduced_probes {1 << 64}")]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse(bad)
        for key, value in (("reduced_fail_highs", 3), ("full_depth_rechecks", 1),
                           ("completed_reduced_fail_lows", 5), ("full_depth_fail_highs", 3)):
            with self.subTest(key=key), self.assertRaises(ValueError):
                parse([line({**stats, key: value})])
        for absent in ("root", "policy", "latency"):
            with self.subTest(absent=absent), self.assertRaisesRegex(ValueError, "requires policy"):
                parse([line()], **{absent: None})

    def test_measure_observes_helper_lmr_with_and_without_counter_overhead(self):
        root_line = ("info string cooperative_root generations 1 helper_nodes 0 helper_probes 0 "
                     "helper_fail_highs 0 main_confirmations 0 confirmed_alpha_raises 0 "
                     "main_nodes 100 main_tasks 1 confirmation_nodes 0 helper_completed_nodes 0 "
                     "helper_aborted_nodes 0 helper_aborted_probes 0 spawn_failures 0 helpers_started 1 "
                     "helper_start_delay_us 0 join_wait_us 0 alpha_gap_sum 0 alpha_gap_max 0 "
                     "physical_helper_threads_started 1 physical_helper_threads_joined 1 "
                     "executor_start_delay_us 0 helper_dispatch_delay_us 0")
        for instrumented in (False, True):
            args = self.measurement_args(yaneuraou=False, evalfile=None)
            args.option = ["T_COOP_HELPER_LMR=1", f"T_COOP_LATENCY={int(instrumented)}"]
            args.provenance.update(counter_overhead_included=False, timing_qualified=True)
            outputs = [root_line, "info string cooperative_root_helper_lmr_policy enabled 1"]
            if instrumented:
                outputs += ["info string cooperative_root_latency enabled 1 " + " ".join(f"{key} 0" for key in COOPERATIVE_ROOT_LATENCY_KEYS),
                            "info string cooperative_root_helper_lmr " + " ".join(f"{key} 0" for key in COOPERATIVE_ROOT_HELPER_LMR_KEYS)]
            outputs += ["info depth 3 score cp 0 nodes 100 nps 1000 pv 7g7f", "bestmove 7g7f"]
            with self.subTest(instrumented=instrumented), patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
                usi.return_value.wait_for.side_effect = lambda token: outputs if token == "bestmove" else [token]
                row = measure(args, 2, ["startpos"])[0]
            self.assertEqual(row["cooperative_root_helper_lmr_policy"], {"enabled": 1})
            self.assertEqual(row["cooperative_root_helper_lmr"] is not None, instrumented)
            self.assertTrue(row["provenance"]["cooperative_helper_lmr_enabled"])
            self.assertEqual(row["provenance"]["counter_overhead_included"], instrumented)
            self.assertEqual(row["provenance"]["timing_qualified"], not instrumented)

    def test_go_command_keeps_time_and_fixed_depth_budgets_distinct(self):
        self.assertEqual(
            go_command(argparse.Namespace(depth=None, movetime=300)), "go movetime 300"
        )
        self.assertEqual(
            go_command(argparse.Namespace(depth=12, movetime=300)), "go depth 12"
        )

    def test_tt_summary_pools_counts_before_computing_rates(self):
        summary = summarize_lazy_tt(
            [
                {
                    "lazy_smp_tt": {
                        "probes": 10,
                        "hits": 4,
                        "attempted": 5,
                        "committed": 4,
                        "same_hash": 2,
                        "equal_depth": 0,
                        "shallower_rejected": 1,
                        "collision_overwrites": 1,
                    }
                },
                {
                    "lazy_smp_tt": {
                        "probes": 30,
                        "hits": 16,
                        "attempted": 15,
                        "committed": 14,
                        "same_hash": 8,
                        "equal_depth": 0,
                        "shallower_rejected": 1,
                        "collision_overwrites": 0,
                    }
                },
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["probes"], 40)
        self.assertEqual(summary["hits"], 20)
        self.assertEqual(summary["hit_rate"], 0.5)
        self.assertEqual(summary["same_hash_rate"], 0.5)
        self.assertEqual(summary["collision_rate"], 0.05)

    def test_overlap_summary_pools_counts_before_computing_rates(self):
        summary = summarize_lazy_overlap(
            [
                {
                    "lazy_smp_overlap": {
                        "sample_rate": 256,
                        "visits": 40,
                        "unique": 20,
                        "within_worker_revisits": 10,
                        "cross_worker_visits": 10,
                        "shared_fingerprints": 5,
                        "dropped": 0,
                        "tt_sampled_stores": 20,
                        "tt_sampled_hits": 10,
                        "tt_local_hits": 6,
                        "tt_foreign_hits": 3,
                        "tt_unknown_hits": 1,
                        "tt_dropped_stores": 0,
                    }
                },
                {
                    "lazy_smp_overlap": {
                        "sample_rate": 256,
                        "visits": 60,
                        "unique": 30,
                        "within_worker_revisits": 12,
                        "cross_worker_visits": 18,
                        "shared_fingerprints": 9,
                        "dropped": 1,
                        "tt_sampled_stores": 30,
                        "tt_sampled_hits": 20,
                        "tt_local_hits": 9,
                        "tt_foreign_hits": 8,
                        "tt_unknown_hits": 3,
                        "tt_dropped_stores": 1,
                    }
                },
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["visits"], 100)
        self.assertEqual(summary["unique"], 50)
        self.assertEqual(summary["cross_worker_visit_rate"], 0.28)
        self.assertEqual(summary["shared_fingerprint_rate"], 0.28)
        self.assertEqual(summary["within_worker_revisit_rate"], 0.22)
        self.assertEqual(summary["drop_rate"], 0.01)
        self.assertEqual(summary["foreign_tt_hit_rate"], 11 / 26)
        self.assertEqual(summary["tt_unknown_hit_rate"], 4 / 30)

    def test_root_work_summary_uses_total_worker_nodes(self):
        summary = summarize_lazy_root_work(
            [
                {
                    "nodes": 100,
                    "lazy_smp": {
                        "discarded_root_nodes": 70,
                        "divergent_discarded_nodes": 40,
                    },
                },
                {
                    "nodes": 300,
                    "lazy_smp": {
                        "discarded_root_nodes": 180,
                        "divergent_discarded_nodes": 60,
                    },
                },
                {"nodes": 999},
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["total_nodes"], 400)
        self.assertEqual(summary["selected_root_nodes"], 150)
        self.assertEqual(summary["discarded_root_node_rate"], 0.625)
        self.assertEqual(summary["divergent_discarded_node_rate"], 0.25)

    def test_selected_tt_reuse_summary_excludes_unselected_workers(self):
        summary = summarize_selected_tt_reuse(
            [
                {
                    "lazy_smp": {
                        "selected_tt_local_hits": 6,
                        "selected_tt_foreign_hits": 3,
                        "selected_tt_unknown_hits": 1,
                    }
                },
                {
                    "lazy_smp": {
                        "selected_tt_local_hits": 9,
                        "selected_tt_foreign_hits": 7,
                        "selected_tt_unknown_hits": 4,
                    }
                },
                {"lazy_smp": {"discarded_root_nodes": 123}},
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["selected_tt_local_hits"], 15)
        self.assertEqual(summary["selected_tt_foreign_hits"], 10)
        self.assertEqual(summary["selected_tt_unknown_hits"], 5)
        self.assertEqual(summary["selected_foreign_tt_hit_rate"], 0.4)
        self.assertEqual(summary["selected_tt_unknown_hit_rate"], 1 / 6)

    def test_root_hint_summary_pools_counts_before_rates(self):
        summary = summarize_lazy_root_hints(
            [
                {
                    "lazy_smp_root_hint": {
                        "publications": 2,
                        "eligible_reads": 3,
                        "applied": 1,
                    }
                },
                {
                    "lazy_smp_root_hint": {
                        "publications": 0,
                        "eligible_reads": 0,
                        "applied": 0,
                    }
                },
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["searches"], 2)
        self.assertEqual(summary["publications"], 2)
        self.assertEqual(summary["eligible_reads"], 3)
        self.assertEqual(summary["applied"], 1)
        self.assertEqual(summary["publication_search_rate"], 0.5)
        self.assertEqual(summary["application_search_rate"], 0.5)
        self.assertEqual(summary["reads_per_publication"], 1.5)
        self.assertEqual(summary["application_rate"], 1 / 3)

    def test_root_hint_zero_denominators_are_not_reported_as_zero_rates(self):
        summary = summarize_lazy_root_hints(
            [
                {
                    "lazy_smp_root_hint": {
                        "publications": 0,
                        "eligible_reads": 0,
                        "applied": 0,
                    }
                }
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertIsNone(summary["reads_per_publication"])
        self.assertIsNone(summary["application_rate"])
        self.assertEqual(format_optional_ratio(None, percentage=True), "n/a")

    def test_required_root_hint_summary_rejects_partial_rows(self):
        with self.assertRaisesRegex(ValueError, "missing diagnostic"):
            summarize_lazy_root_hints(
                [
                    {
                        "lazy_smp_root_hint": {
                            "publications": 1,
                            "eligible_reads": 1,
                            "applied": 1,
                        }
                    },
                    {},
                ],
                require_all=True,
            )


class CooperativePrestartTests(unittest.TestCase):
    @staticmethod
    def root(workers=4):
        return dict(workers=workers, helpers=workers - 1, generations=2,
                    helpers_started=2 * (workers - 1), helper_nodes=30,
                    helper_probes=3, helper_fail_highs=1, main_nodes=100)

    @staticmethod
    def basic():
        return dict(passes=1, first_beta_cuts=1, first_aborts=0, helper_nodes=20)

    @staticmethod
    def observer():
        return dict(helper_jobs_before_first_return=2, helper_completed_probes_before_first_return=2,
                    helper_fail_highs_before_first_return=1, first_main_nodes=15,
                    first_main_us=80, first_beta_helper_nodes=5)

    @staticmethod
    def line(values, *, diagnostic=False):
        prefix = "info string cooperative_root_prestart" + ("_diag" if diagnostic else "")
        return prefix + " " + " ".join(f"{key} {value}" for key, value in values.items())

    def test_prestart_and_observer_are_optional_and_have_distinct_prefixes(self):
        self.assertIsNone(parse_cooperative_root_prestart([], cooperative_root=None))
        self.assertIsNone(parse_cooperative_root_prestart_diag([], cooperative_root=None, prestart=None))
        basic, observer = self.basic(), self.observer()
        lines = [self.line(observer, diagnostic=True), self.line(basic)]
        for workers in (4, 8):
            with self.subTest(workers=workers):
                root = self.root(workers)
                self.assertEqual(parse_cooperative_root_prestart(lines, cooperative_root=root), basic)
                self.assertEqual(parse_cooperative_root_prestart_diag(lines, cooperative_root=root, prestart=basic), observer)
        zero = dict.fromkeys(COOPERATIVE_ROOT_PRESTART_KEYS, 0)
        self.assertEqual(parse_cooperative_root_prestart([self.line(zero)], cooperative_root=self.root()), zero)
        empty = dict.fromkeys(COOPERATIVE_ROOT_PRESTART_DIAG_KEYS, 0)
        self.assertEqual(parse_cooperative_root_prestart_diag([self.line(empty, diagnostic=True)], cooperative_root=self.root(), prestart=zero), empty)

    def test_prestart_rejects_wrong_group_or_inconsistent_intrinsic_census(self):
        basic = self.basic()
        for root in (None, {}, {**self.root(), "workers": 2}, {**self.root(), "workers": 16}):
            with self.subTest(root=root), self.assertRaises(ValueError):
                parse_cooperative_root_prestart([self.line(basic)], cooperative_root=root)
        for key, value in (("passes", 3), ("first_beta_cuts", 2), ("first_aborts", 1), ("helper_nodes", 31)):
            with self.subTest(key=key), self.assertRaises(ValueError):
                parse_cooperative_root_prestart([self.line({**basic, key: value})], cooperative_root=self.root())
        for key in COOPERATIVE_ROOT_PRESTART_KEYS[1:]:
            with self.subTest(no_pass_key=key), self.assertRaises(ValueError):
                parse_cooperative_root_prestart([self.line({**dict.fromkeys(COOPERATIVE_ROOT_PRESTART_KEYS, 0), key: 1})], cooperative_root=self.root())

    def test_prestart_observer_rejects_orphan_and_nonprivate_subsets(self):
        basic, observer, root = self.basic(), self.observer(), self.root()
        for absent in ("root", "prestart"):
            with self.subTest(absent=absent), self.assertRaises(ValueError):
                parse_cooperative_root_prestart_diag([self.line(observer, diagnostic=True)],
                    cooperative_root=None if absent == "root" else root,
                    prestart=None if absent == "prestart" else basic)
        changes = (("helper_jobs_before_first_return", 4), ("helper_completed_probes_before_first_return", 4),
                   ("helper_fail_highs_before_first_return", 2), ("helper_completed_probes_before_first_return", 0),
                   ("helper_jobs_before_first_return", 0),
                   ("first_main_nodes", 101), ("first_beta_helper_nodes", 21))
        for key, value in changes:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                parse_cooperative_root_prestart_diag([self.line({**observer, key: value}, diagnostic=True)], cooperative_root=root, prestart=basic)
        with self.assertRaisesRegex(ValueError, "started jobs"):
            parse_cooperative_root_prestart_diag([self.line(observer, diagnostic=True)], cooperative_root={**root, "helpers_started": 1}, prestart=basic)
        with self.assertRaisesRegex(ValueError, "first beta"):
            parse_cooperative_root_prestart_diag([self.line(observer, diagnostic=True)], cooperative_root=root, prestart={**basic, "first_beta_cuts": 0})
        zero = dict.fromkeys(COOPERATIVE_ROOT_PRESTART_KEYS, 0)
        for key in ("first_main_nodes", "first_main_us"):
            with self.subTest(no_pass_key=key), self.assertRaises(ValueError):
                parse_cooperative_root_prestart_diag([self.line({**dict.fromkeys(COOPERATIVE_ROOT_PRESTART_DIAG_KEYS, 0), key: 1}, diagnostic=True)], cooperative_root=root, prestart=zero)

    def test_prestart_records_are_strict_ordered_ascii_u64(self):
        for diagnostic, values in ((False, self.basic()), (True, self.observer())):
            def parse(lines):
                if diagnostic:
                    return parse_cooperative_root_prestart_diag(lines, cooperative_root=self.root(), prestart=self.basic())
                return parse_cooperative_root_prestart(lines, cooperative_root=self.root())
            line = self.line(values, diagnostic=diagnostic)
            malformed = ([line, line], [line + " extra 0"], [line.rsplit(" ", 2)[0]],
                         [self.line(dict(reversed(list(values.items()))), diagnostic=diagnostic)],
                         [line.split(" ")[0] + " string cooperative_root_prestart" + ("_diag" if diagnostic else "")])
            for lines in malformed:
                with self.subTest(diagnostic=diagnostic, lines=lines), self.assertRaises(ValueError):
                    parse(lines)
            for key in values:
                for token in ("-1", "+1", "１", "1.0", "1e1", str(1 << 64)):
                    with self.subTest(diagnostic=diagnostic, key=key, token=token), self.assertRaises(ValueError):
                        parse([self.line({**values, key: token}, diagnostic=diagnostic)])
        maximum = (1 << 64) - 1
        basic = {**self.basic(), "helper_nodes": maximum}
        self.assertEqual(parse_cooperative_root_prestart([self.line(basic)], cooperative_root={**self.root(), "helper_nodes": maximum}), basic)
        observer = {**self.observer(), "first_main_us": maximum}
        self.assertEqual(parse_cooperative_root_prestart_diag([self.line(observer, diagnostic=True)], cooperative_root=self.root(), prestart=self.basic()), observer)

    def test_measure_separates_intrinsic_prestart_and_observer_timing(self):
        for diagnostic in (False, True):
            with self.subTest(diagnostic=diagnostic):
                args = ThreadOptionsTest.measurement_args(yaneuraou=False, evalfile=None)
                args.provenance = {"counter_overhead_included": False, "timing_qualified": True}
                args.option = ["T_COOP_PRESTART=1", f"T_COOP_PRESTART_DIAG={int(diagnostic)}"]
                outputs = [ThreadOptionsTest.cooperative_root_line(4),
                           "info string cooperative_root_group workers 4 helpers 3", self.line(self.basic())]
                if diagnostic:
                    outputs.append(self.line(self.observer(), diagnostic=True))
                outputs += ["info depth 3 score cp 0 nodes 130 nps 1000 pv 7g7f", "bestmove 7g7f"]
                with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
                    usi.return_value.wait_for.side_effect = lambda token: outputs if token == "bestmove" else [token]
                    row = measure(args, 4, ["startpos"])[0]
                self.assertEqual(row["cooperative_root_prestart"], self.basic())
                self.assertEqual(row["cooperative_root_prestart_diag"], self.observer() if diagnostic else None)
                self.assertTrue(row["provenance"]["cooperative_prestart_enabled"])
                self.assertEqual(row["provenance"]["cooperative_prestart_overhead_included"], diagnostic)
                self.assertEqual(row["provenance"]["counter_overhead_included"], diagnostic)
                self.assertEqual(row["provenance"]["timing_qualified"], not diagnostic)


class FixedNodeMeasurementTests(unittest.TestCase):
    def test_positive_node_limit_is_strict_ascii_u64_and_builds_go_command(self):
        for token, expected in (("1", 1), ("00010", 10), ("0" * 5000 + "1", 1), (str((1 << 64) - 1), (1 << 64) - 1)):
            with self.subTest(token=token):
                self.assertEqual(positive_u64(token), expected)
                self.assertEqual(go_command(argparse.Namespace(depth=None, nodes=expected, movetime=300)), f"go nodes {expected}")
        for token in ("0", "-1", "+1", "１", "1.0", "1e3", "", " 1", str(1 << 64), "9" * 5000):
            with self.subTest(token=token[:30]), self.assertRaises(argparse.ArgumentTypeError):
                positive_u64(token)

    def test_node_cli_rejects_conflicting_budgets_before_reading_inputs(self):
        for flags in (["--nodes", "10", "--depth", "3"], ["--depth", "3", "--nodes", "10"],
                      ["--nodes", "10", "--movetime", "300"], ["--nodes", "0"], ["--nodes", str(1 << 64)]):
            with self.subTest(flags=flags), patch("sys.argv", ["nps_threads.py", "fake-engine", *flags]), patch("nps_threads.sys.stderr"), patch("nps_threads.read_positions") as read:
                with self.assertRaises(SystemExit) as error:
                    nps_main()
                self.assertEqual(error.exception.code, 2)
                read.assert_not_called()

    def test_node_namespace_validation_precedes_engine_start(self):
        for depth, nodes in ((3, 10), (None, 0), (None, -1), (None, 1 << 64), (None, True), (None, "10")):
            with self.subTest(depth=depth, nodes=nodes), patch("nps_threads.Usi") as usi:
                args = ThreadOptionsTest.measurement_args(yaneuraou=False, evalfile=None)
                args.depth, args.nodes = depth, nodes
                with self.assertRaises(ValueError):
                    measure(args, 1, ["startpos"])
                usi.assert_not_called()

    def test_node_measurement_records_overshoot_without_timing_qualification(self):
        for limit, overshoot in ((90, 10), (100, 0), (200, 0)):
            with self.subTest(limit=limit):
                args = ThreadOptionsTest.measurement_args(yaneuraou=False, evalfile=None)
                args.depth, args.nodes = None, limit
                args.provenance.update(timing_qualified=True)
                with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
                    usi.return_value.wait_for.side_effect = ThreadOptionsTest.fake_response
                    row = measure(args, 1, ["startpos"])[0]
                usi.return_value.send.assert_any_call(f"go nodes {limit}")
                self.assertEqual(row["nodes"], 100)
                self.assertEqual(row["node_limit_overshoot"], overshoot)
                self.assertEqual(row["provenance"]["node_limit"], limit)
                self.assertEqual(row["provenance"]["measurement_phase"], "fixed_node_work_diagnostic")
                self.assertEqual(row["provenance"]["budget"], f"go nodes {limit}")
                self.assertFalse(row["provenance"]["timing_qualified"])


class CooperativePrestartCapTests(unittest.TestCase):
    @staticmethod
    def cap():
        return dict(limit=1, passes=1, initial_claims=1, park_calls=3,
                    resumed_helpers=2, cancelled_helpers=1)

    @staticmethod
    def line(values):
        return "info string cooperative_root_prestart_cap " + " ".join(f"{key} {value}" for key, value in values.items())

    def parse(self, lines, *, root=None, prestart=None):
        return parse_cooperative_root_prestart_cap(lines, cooperative_root=root, prestart=prestart)

    def test_cap_is_optional_distinct_from_observer_and_accepts_four_eight_groups(self):
        self.assertIsNone(self.parse([]))
        cap = self.cap()
        basic = CooperativePrestartTests.basic()
        lines = [self.line(cap), CooperativePrestartTests.line(basic)]
        for count in (4, 8):
            with self.subTest(count=count):
                root = CooperativePrestartTests.root(count)
                self.assertEqual(self.parse(lines, root=root, prestart=basic), cap)
                self.assertEqual(parse_cooperative_root_prestart(lines, cooperative_root=root), basic)
                self.assertIsNone(parse_cooperative_root_prestart_diag(lines, cooperative_root=root, prestart=basic))
        zero = dict.fromkeys(COOPERATIVE_ROOT_PRESTART_CAP_KEYS, 0)
        zero["limit"] = 8
        self.assertEqual(self.parse([self.line(zero)], root=CooperativePrestartTests.root(), prestart=dict.fromkeys(COOPERATIVE_ROOT_PRESTART_KEYS, 0)), zero)
        maximum = {**cap, "park_calls": (1 << 64) - 1}
        self.assertEqual(self.parse([self.line(maximum)], root=CooperativePrestartTests.root(), prestart=basic), maximum)

    def test_cap_rejects_orphan_wrong_group_and_census_bounds(self):
        cap, root, basic = self.cap(), CooperativePrestartTests.root(), CooperativePrestartTests.basic()
        for other_root, other_basic in ((None, basic), (root, None), ({**root, "workers": 2}, basic), ({**root, "workers": 16}, basic)):
            with self.subTest(root=other_root, prestart=other_basic), self.assertRaises(ValueError):
                self.parse([self.line(cap)], root=other_root, prestart=other_basic)
        for updates in ({"limit": 0}, {"limit": 9}, {"passes": 2}, {"initial_claims": 2},
                        {"resumed_helpers": 6, "park_calls": 7}, {"cancelled_helpers": 5, "park_calls": 7}, {"park_calls": 2}):
            with self.subTest(updates=updates), self.assertRaises(ValueError):
                self.parse([self.line({**cap, **updates})], root=root, prestart=basic)
        zero = dict.fromkeys(COOPERATIVE_ROOT_PRESTART_CAP_KEYS, 0)
        zero["limit"] = 1
        for key in COOPERATIVE_ROOT_PRESTART_CAP_KEYS[2:]:
            with self.subTest(no_pass_key=key), self.assertRaises(ValueError):
                self.parse([self.line({**zero, key: 1})], root=root, prestart=dict.fromkeys(COOPERATIVE_ROOT_PRESTART_KEYS, 0))

    def test_cap_keys_are_unique_ordered_ascii_unsigned_u64(self):
        cap, root, basic = self.cap(), CooperativePrestartTests.root(), CooperativePrestartTests.basic()
        line = self.line(cap)
        for lines in ([line, line], [line + " extra 0"], [self.line(dict(reversed(list(cap.items()))))],
                      [line.rsplit(" ", 2)[0]], ["info string cooperative_root_prestart_cap"]):
            with self.subTest(lines=lines), self.assertRaises(ValueError):
                self.parse(lines, root=root, prestart=basic)
        for key in cap:
            for token in ("-1", "+1", "１", "1.0", "1e1", str(1 << 64)):
                with self.subTest(key=key, token=token), self.assertRaises(ValueError):
                    self.parse([self.line({**cap, key: token})], root=root, prestart=basic)

    def test_actual_cap_is_policy_without_observer_overhead_in_time_and_node_lanes(self):
        for node_limit in (None, 100):
            for diagnostic in (False, True):
                with self.subTest(node_limit=node_limit, diagnostic=diagnostic):
                    args = ThreadOptionsTest.measurement_args(yaneuraou=False, evalfile=None)
                    args.depth, args.nodes = None, node_limit
                    args.provenance = {"counter_overhead_included": False, "timing_qualified": True}
                    args.option = ["T_COOP_PRESTART=1", "T_COOP_PRESTART_CAP=1", f"T_COOP_PRESTART_DIAG={int(diagnostic)}"]
                    outputs = [ThreadOptionsTest.cooperative_root_line(4),
                               "info string cooperative_root_group workers 4 helpers 3",
                               CooperativePrestartTests.line(CooperativePrestartTests.basic()), self.line(self.cap())]
                    if diagnostic:
                        outputs.append(CooperativePrestartTests.line(CooperativePrestartTests.observer(), diagnostic=True))
                    outputs += ["info depth 3 score cp 0 nodes 130 nps 1000 pv 7g7f", "bestmove 7g7f"]
                    with patch("nps_threads.Usi") as usi, patch("nps_threads.sys.stderr"):
                        usi.return_value.wait_for.side_effect = lambda token: outputs if token == "bestmove" else [token]
                        row = measure(args, 4, ["startpos"])[0]
                    self.assertEqual(row["cooperative_root_prestart_cap"], self.cap())
                    self.assertTrue(row["provenance"]["cooperative_prestart_cap_enabled"])
                    self.assertEqual(row["provenance"]["counter_overhead_included"], diagnostic)
                    self.assertEqual(row["provenance"]["cooperative_prestart_overhead_included"], diagnostic)
                    self.assertEqual(row["provenance"]["timing_qualified"], node_limit is None and not diagnostic)
                    if node_limit is not None:
                        self.assertEqual(row["node_limit_overshoot"], 30)


if __name__ == "__main__":
    unittest.main()
