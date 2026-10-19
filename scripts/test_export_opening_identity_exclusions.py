#!/usr/bin/env python3
"""Synthetic owner-export contracts; never read evaluation or holdout data."""
import contextlib
import hashlib
import io
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import export_opening_identity_exclusions as export
from freeze_material_gate_openings import ROLES, identity_hash, load_exclusions

BASE = "4k4/9/9/9/9/9/9/9/4K4 b P2pR 2"
SAME = "4k4/9/9/9/9/9/9/9/4K4 b RP2p 99"
SIDE = BASE.replace(" b ", " w ")
HAND = BASE.replace("P2pR", "P3pR")
BOARD = BASE.replace("4k4", "3k5", 1)
PRIVATE = "private-training-label-934721"


class IdentityExportTests(unittest.TestCase):
    def args(self, root, inputs, *, role="diagnostic", complete=True, owner="fixture owner", output="identities.json", extra=()):
        argv = ["--role", role, "--output", str(root / output)]
        for format_name, path in inputs:
            argv += ["--input", format_name, str(path)]
        if complete:
            argv.append("--attest-complete-for-scope")
        if owner is not None:
            argv += ["--attested-by", owner]
        return export.parse_args(argv + list(extra))

    def assert_clean_failure(self, args, errors=(ValueError, OSError)):
        with self.assertRaises(errors):
            export.export_manifest(args)
        self.assertFalse(args.output.exists())
        self.assertEqual(list(args.output.parent.glob(".opening-identities-*")), [])

    def test_three_formats_hash_the_parsed_stream_and_deduplicate_final_identities(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            inputs = {
                "sfen": f"# synthetic only\n\n{BASE}\n{SAME}\n{SIDE}\n{HAND}\n{BOARD}\n".encode(),
                "tsv": ("label\tsfen\n" + "".join(f"{PRIVATE}\t{sfen}\n" for sfen in (BASE, SAME, SIDE, HAND, BOARD))).encode(),
                "jsonl": "".join(json.dumps({"sfen": sfen, "label": PRIVATE, "score": 314159}) + "\n" for sfen in (BASE, SAME, SIDE, HAND, BOARD)).encode(),
            }
            expected = {identity_hash(sfen) for sfen in (BASE, SIDE, HAND, BOARD)}
            self.assertEqual(identity_hash(BASE), identity_hash(SAME))
            self.assertEqual(len(expected), 4)
            for format_name, payload in inputs.items():
                path = root / format_name
                path.write_bytes(payload)
                values, source, stamp = export.read_source(path, format_name, column=1 if format_name == "tsv" else 0, header=format_name == "tsv")
                self.assertEqual(values, expected)
                self.assertEqual(source["sha256"], hashlib.sha256(payload).hexdigest())
                self.assertEqual(source["bytes"], len(payload))
                self.assertEqual(source["records"], 5)
                self.assertEqual(source["unique_identities"], 4)
                self.assertEqual(source["path"], str(path.resolve()))
                self.assertEqual(stamp.resolved, path.resolve())
                extra = ("--tsv-sfen-column", "1", "--tsv-header") if format_name == "tsv" else ()
                args = self.args(root, [(format_name, path)], output=format_name + ".json", extra=extra)
                document = export.export_manifest(args)
                self.assertEqual(document["identities"], sorted(expected))
                self.assertEqual(document["duplicate_records"], 1)
                self.assertEqual(json.loads(args.output.read_bytes()), document)
                text = args.output.read_text()
                for secret in (BASE, SAME, PRIVATE, '"score":', '"sfen":'):
                    self.assertNotIn(secret, text)

    def test_combined_formats_deduplicate_across_distinct_sources(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            plain, table, jsonl = root / "plain", root / "table", root / "jsonl"
            plain.write_text(BASE + "\n" + SAME + "\n")
            table.write_text("label\tsfen\n" + PRIVATE + "\t" + SIDE + "\n")
            jsonl.write_text(json.dumps({"sfen": SAME, "label": PRIVATE}) + "\n")
            args = self.args(root, [("sfen", plain), ("tsv", table), ("jsonl", jsonl)],
                             extra=("--tsv-sfen-column", "1", "--tsv-header"))
            document = export.export_manifest(args)
            self.assertEqual(document["records"], 4)
            self.assertEqual(document["unique_identities"], 2)
            self.assertEqual(document["duplicate_records"], 2)
            self.assertEqual(len(document["sources"]), 3)
            self.assertEqual(document["identities"], sorted({identity_hash(BASE), identity_hash(SIDE)}))

    def test_frozen_opening_manifest_is_checked_before_export(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest = root / "openings.json"
            document = {
                "schema": "sekirei.independent-material-openings.v1",
                "identity_scheme": export.IDENTITY,
                "positions": [
                    {"sfen": BASE, "identity_sha256": identity_hash(BASE)},
                    {"sfen": SAME, "identity_sha256": identity_hash(SAME)},
                    {"sfen": SIDE, "identity_sha256": identity_hash(SIDE)},
                ],
            }
            manifest.write_text(json.dumps(document), encoding="utf-8")
            values, source, _ = export.read_source(manifest, "opening-manifest")
            self.assertEqual(values, {identity_hash(BASE), identity_hash(SIDE)})
            self.assertEqual(source["records"], 3)
            result = export.export_manifest(
                self.args(root, [("opening-manifest", manifest)])
            )
            self.assertEqual(result["unique_identities"], 2)
            document["positions"][0]["identity_sha256"] = "0" * 64
            manifest.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "invalid opening-manifest"):
                export.read_source(manifest, "opening-manifest")

    def test_tsv_options_require_a_tsv_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "plain"
            source.write_text(BASE + "\n")
            for extra in (("--tsv-header",), ("--tsv-sfen-column", "1")):
                with self.subTest(extra=extra):
                    self.assert_clean_failure(self.args(root, [("sfen", source)], extra=extra))

    def test_three_attested_roles_qualify_existing_loader_but_two_or_default_false_do_not(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "final.sfen"
            source.write_text(BASE + "\n")
            outputs = []
            for role in sorted(ROLES):
                args = self.args(root, [("sfen", source)], role=role, output=role + ".json")
                document = export.export_manifest(args)
                self.assertTrue(document["complete_for_scope"])
                self.assertEqual(document["attestation"]["by"], "fixture owner")
                outputs.append(args.output)
            values, records = load_exclusions(outputs)
            self.assertEqual(values, {identity_hash(BASE)})
            self.assertEqual(len(records), 3)
            with self.assertRaisesRegex(ValueError, "missing complete"):
                load_exclusions(outputs[:2])
            args = self.args(root, [("sfen", source)], role=sorted(ROLES)[0], complete=False, owner=None, output="unattested.json")
            document = export.export_manifest(args)
            self.assertFalse(document["complete_for_scope"])
            self.assertIsNone(document["attestation"])
            with self.assertRaisesRegex(ValueError, "unqualified"):
                load_exclusions([args.output, *outputs[1:]])

    def test_attestation_requires_both_flag_and_nonempty_single_line_owner(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "final.sfen"
            source.write_text(BASE + "\n")
            for complete, owner in ((True, None), (False, "fixture owner"), (True, "  "),
                                    (True, "owner\nprivate"), (True, "owner\x7fprivate"),
                                    (True, "owner\u2028private"), (True, "owner\u2029private"),
                                    (True, "owner\x85private"), (True, "owner\n"),
                                    (True, "\towner"), (True, "\u2028owner"),
                                    (True, "owner\u2029"), (True, "owner\x85")):
                with self.subTest(complete=complete, owner=owner):
                    self.assert_clean_failure(self.args(root, [("sfen", source)], complete=complete, owner=owner))
            args = self.args(root, [("sfen", source)], owner="  fixture owner  ")
            self.assertEqual(export.export_manifest(args)["attestation"]["by"], "fixture owner")

    def test_cli_requires_explicit_role_input_and_output(self):
        for argv in ([], ["--role", "diagnostic"], ["--role", "other", "--input", "sfen", "x", "--output", "y"]):
            with self.subTest(argv=argv), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                export.parse_args(argv)
            self.assertEqual(error.exception.code, 2)

    def test_programmatic_export_rejects_missing_inputs_or_unknown_role(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.write_text(BASE + "\n")
            for field, value in (("input", []), ("role", "unknown")):
                args = self.args(root, [("sfen", source)])
                setattr(args, field, value)
                with self.subTest(field=field):
                    self.assert_clean_failure(args)

    def test_invalid_records_never_create_an_output_or_echo_private_content(self):
        invalid = [
            ("sfen", b"", ()),
            ("tsv", b"\n", ()),
            ("jsonl", b"\n", ()),
            ("sfen", b"\xff\xfe\n", ()),
            ("sfen", (BASE + "\n").encode() + b"\xff\n", ()),
            ("sfen", (PRIVATE + " " + BASE + "\n").encode(), ()),
            ("sfen", ("position sfen " + BASE + "\n").encode(), ()),
            ("sfen", ("sfen " + BASE + "\n").encode(), ()),
            ("sfen", b"position startpos moves 7g7f\n", ()),
            ("sfen", BASE.replace(" b ", " x ").encode(), ()),
            ("sfen", b"\n # only comments\n", ()),
            ("jsonl", json.dumps({"label": PRIVATE}).encode(), ()),
            ("jsonl", json.dumps({"sfen": 17, "label": PRIVATE}).encode(), ()),
            ("jsonl", json.dumps([BASE, PRIVATE]).encode(), ()),
            ("jsonl", ('{"sfen":' + json.dumps(BASE) + ',"sfen":' + json.dumps(SAME) + '}').encode(), ()),
            ("jsonl", ('{"sfen":' + json.dumps(BASE) + ',"meta":{"label":1,"label":2}}').encode(), ()),
            ("jsonl", b'{"sfen":"' + PRIVATE.encode() + b'"', ()),
            *[("jsonl", ('{"sfen":' + json.dumps(BASE) + ',"label":' + value + '}').encode(), ())
              for value in ("NaN", "Infinity", "-Infinity")],
            ("tsv", (PRIVATE + "\n").encode(), ("--tsv-sfen-column", "1")),
            ("tsv", ("label\tSFEN\n" + PRIVATE + "\t" + BASE + "\n").encode(), ("--tsv-sfen-column", "1", "--tsv-header")),
            ("tsv", b"label\tsfen\n", ("--tsv-sfen-column", "1", "--tsv-header")),
            ("tsv", (PRIVATE + "\t" + BASE + "\n").encode(), ("--tsv-sfen-column", "-1")),
            ("tsv", (PRIVATE + "\t" + BASE + "\n").encode(), ("--tsv-sfen-column", "64")),
        ]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for number, (format_name, payload, extra) in enumerate(invalid):
                source = root / "source"
                source.write_bytes(payload)
                args = self.args(root, [(format_name, source)], extra=extra)
                with self.subTest(case=number), self.assertRaises((ValueError, OSError)) as error:
                    export.export_manifest(args)
                self.assertNotIn(PRIVATE, str(error.exception))
                self.assertNotIn(BASE, str(error.exception))
                self.assertFalse(args.output.exists())
                self.assertEqual(list(root.glob(".opening-identities-*")), [])

    def test_oversized_records_and_invalid_source_configuration_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.write_text(PRIVATE * 20 + "\n")
            with patch.object(export, "MAX_RECORD_BYTES", 64):
                self.assert_clean_failure(self.args(root, [("sfen", source)]))
            source.write_text(BASE + "\n")
            for format_name, column, header in (("usi", 0, False), ("sfen", -1, False), ("sfen", 64, False), ("jsonl", 0, True)):
                with self.subTest(format=format_name, column=column, header=header), self.assertRaises(ValueError):
                    export.read_source(source, format_name, column=column, header=header)
            for path in (root / "missing", root):
                with self.subTest(path=path):
                    self.assert_clean_failure(self.args(root, [("sfen", path)]))

    def test_source_mutation_while_streaming_prevents_publication(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source.sfen"
            source.write_text(BASE + "\n")
            actual_identity = export.identity_hash
            calls = 0
            def mutate_once(sfen):
                nonlocal calls
                calls += 1
                # Mutate only the first parsed record to avoid an unbounded stream.
                if calls == 1:
                    with source.open("ab") as output:
                        output.write((SIDE + "\n").encode())
                return actual_identity(sfen)
            with patch.object(export, "identity_hash", side_effect=mutate_once), self.assertRaisesRegex(ValueError, "changed"):
                export.export_manifest(self.args(root, [("sfen", source)]))
            self.assertFalse((root / "identities.json").exists())

    def test_earlier_source_change_and_symlink_retarget_during_later_read_are_rejected(self):
        for retarget in (False, True):
            with self.subTest(retarget=retarget), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                first, second, other = root / "first", root / "second", root / "other"
                first.write_text(BASE + "\n")
                second.write_text(SIDE + "\n")
                other.write_text(BASE + "\n")
                requested = root / "link" if retarget else first
                if retarget:
                    requested.symlink_to(first)
                original_read = export.read_source
                def read(path, *args, **kwargs):
                    if path == second:
                        if retarget:
                            requested.unlink()
                            requested.symlink_to(other)
                        else:
                            first.write_text(HAND + "\n")
                    return original_read(path, *args, **kwargs)
                with patch.object(export, "read_source", side_effect=read), self.assertRaisesRegex(ValueError, "changed"):
                    export.export_manifest(self.args(root, [("sfen", requested), ("sfen", second)]))
                self.assertFalse((root / "identities.json").exists())

    def test_duplicate_source_names_symlinks_and_hardlinks_are_not_independent_inputs(self):
        for alias_kind in ("same", "symlink", "hardlink"):
            with self.subTest(alias_kind=alias_kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source = root / "source"
                original = (BASE + "\n").encode()
                source.write_bytes(original)
                alias = source if alias_kind == "same" else root / "alias"
                if alias_kind == "symlink":
                    alias.symlink_to(source)
                elif alias_kind == "hardlink":
                    os.link(source, alias)
                with self.assertRaisesRegex(ValueError, "duplicate"):
                    export.export_manifest(self.args(root, [("sfen", source), ("sfen", alias)]))
                self.assertEqual(source.read_bytes(), original)
                self.assertFalse((root / "identities.json").exists())

    def test_existing_output_and_source_aliases_are_never_overwritten(self):
        for target_kind in ("existing", "source", "hardlink", "symlink", "dangling"):
            with self.subTest(target_kind=target_kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source, output = root / "source", root / "output"
                original = (BASE + "\n").encode()
                source.write_bytes(original)
                if target_kind == "existing":
                    output.write_bytes(b"keep existing result")
                elif target_kind == "source":
                    output = source
                elif target_kind == "hardlink":
                    os.link(source, output)
                elif target_kind == "symlink":
                    output.symlink_to(source)
                else:
                    output.symlink_to(root / "missing")
                before = output.read_bytes() if output.exists() else None
                args = self.args(root, [("sfen", source)], output=output.name)
                with self.assertRaisesRegex(ValueError, "already exists"):
                    export.export_manifest(args)
                self.assertEqual(source.read_bytes(), original)
                self.assertTrue(os.path.lexists(output))
                if before is not None:
                    self.assertEqual(output.read_bytes(), before)

    def test_source_change_during_manifest_fsync_prevents_publication(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.write_text(BASE + "\n")
            args = self.args(root, [("sfen", source)])
            actual_fsync = os.fsync
            def mutate(fd):
                actual_fsync(fd)
                source.write_text(HAND + "\n# changed after export temp was written\n")
            with patch.object(export.os, "fsync", side_effect=mutate):
                self.assert_clean_failure(args, errors=ValueError)

    def test_atomic_publication_preserves_racing_output_and_cleans_temporary_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            output = root / "output"
            actual_link = os.link
            def race(temporary, destination):
                self.assertEqual(stat.S_IMODE(Path(temporary).stat().st_mode), 0o600)
                self.assertEqual(json.loads(Path(temporary).read_bytes()), {"complete": True})
                Path(destination).write_bytes(b"concurrent result")
                actual_link(temporary, destination)
            with patch.object(export.os, "link", side_effect=race), self.assertRaises(FileExistsError):
                export._publish(output, b'{"complete":true}\n')
            self.assertEqual(output.read_bytes(), b"concurrent result")
            self.assertEqual(list(root.glob(".opening-identities-*")), [])

    def test_publication_failures_clean_temporary_files(self):
        for failing in ("fsync", "link"):
            with self.subTest(failing=failing), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                with patch.object(export.os, failing, side_effect=OSError("synthetic failure")), self.assertRaises(OSError):
                    export._publish(root / "output", b"complete payload")
                self.assertFalse((root / "output").exists())
                self.assertEqual(list(root.glob(".opening-identities-*")), [])

    def test_cli_summaries_and_failures_never_emit_records_or_launch_processes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source.jsonl"
            source.write_text(json.dumps({"sfen": BASE, "label": PRIVATE}) + "\n")
            argv = ["--role", "diagnostic", "--input", "jsonl", str(source), "--output", str(root / "output")]
            stdout, stderr = io.StringIO(), io.StringIO()
            with patch("subprocess.run", side_effect=AssertionError("unexpected process")) as run, patch("subprocess.Popen", side_effect=AssertionError("unexpected process")) as popen, contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                self.assertEqual(export.main(argv), 0)
                source.write_text('{"label":' + json.dumps(PRIVATE) + '}\n')
                argv[-1] = str(root / "invalid-output")
                self.assertEqual(export.main(argv), 2)
                run.assert_not_called()
                popen.assert_not_called()
            for text in (stdout.getvalue(), stderr.getvalue()):
                self.assertNotIn(BASE, text)
                self.assertNotIn(PRIVATE, text)
            self.assertEqual(json.loads(stdout.getvalue())["unique_identities"], 1)
            self.assertIn("identity export failed", stderr.getvalue())
            self.assertFalse((root / "invalid-output").exists())


if __name__ == "__main__":
    unittest.main()
