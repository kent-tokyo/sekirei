#!/usr/bin/env python3
"""Fail-closed tests for independent material-opening evidence."""
import argparse
import contextlib
import copy
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from freeze_material_gate_openings import (
    IDENTITY, ROLES, canonical_identity, digest, identity_hash,
    load_exclusions, parse_trajectories, prepare, select_openings, validate_corpus,
)
from generate_search_diagnostic import START, apply
from run_ab_match import main as match_main, parse_args as match_args


class IndependentOpeningsTest(unittest.TestCase):
    def config(self):
        return {"seed": 2026101408, "trajectories": 3, "random_plies": 2,
                "min_ply": 2, "max_ply": 2, "depth": 2, "nodes": 128, "hash_mb": 1}

    def protocol(self):
        rows = ["sekirei.material-gate-trajectories.v1\t0.4.1"]
        for index, moves in enumerate(["7g7f 3c3d", "2g2f 8c8d", "7g7f 8c8d"]):
            sfen = START
            for move in moves.split():
                sfen = apply(sfen, move)
            rows.append(f"position\t{index}\t2\t{sfen}\tstartpos moves {moves}")
        return "\n".join(rows + ["complete\t3\t3"]) + "\n"

    def exclusions(self, root):
        paths = []
        for role in sorted(ROLES):
            path = root / (role + ".json")
            path.write_text(json.dumps({"schema": "sekirei.opening-identity-exclusions.v1",
                "identity_scheme": IDENTITY, "role": role, "complete_for_scope": True,
                "identities": [identity_hash(START)],
                "sources": [{"path": "fixture-only/" + role, "sha256": digest(role.encode())}]}))
            paths.append(path)
        return paths

    def fixture(self, root):
        generator, runner = root / "generator-input", root / "runner-input"
        generator.write_bytes(b"generator-fixture")
        runner.write_bytes(b"runner-fixture")
        config = self.config()
        args = argparse.Namespace(**config, count=2, timeout=10, source_ref="1" * 40,
            generator=generator, match_binary=runner, output=root / "frozen",
            exclusion_manifest=self.exclusions(root))
        completed = [argparse.Namespace(stdout=self.protocol(), stderr=""),
                     argparse.Namespace(stdout="validated 2 openings", stderr="")]
        with patch("freeze_material_gate_openings.subprocess.run", side_effect=completed):
            manifest = prepare(args)
        return args, manifest

    def test_identity_ignores_move_number_and_canonicalizes_hand_order(self):
        base = "4k4/9/9/9/9/9/9/9/4K4 b "
        self.assertEqual(identity_hash(base + "P2pR 2"), identity_hash(base + "RP2p 99"))
        self.assertNotEqual(identity_hash(START), identity_hash(START.replace(" b ", " w ")))
        for sfen in ["bad", START.replace(" b ", " x "), base + "PP 1", base + "P2 1", base + "0P 1"]:
            with self.assertRaises(ValueError):
                canonical_identity(sfen)

    def test_missing_or_unattested_owner_scope_prevents_generation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            paths = self.exclusions(root)
            with self.assertRaisesRegex(ValueError, "missing complete"):
                load_exclusions(paths[:-1])
            document = json.loads(paths[0].read_text())
            document["complete_for_scope"] = False
            paths[0].write_text(json.dumps(document))
            with self.assertRaisesRegex(ValueError, "unqualified"):
                load_exclusions(paths)
            with patch("freeze_material_gate_openings.subprocess.run") as run:
                args = argparse.Namespace(exclusion_manifest=paths, output=root / "not-created")
                with self.assertRaises(ValueError):
                    prepare(args)
                run.assert_not_called()
                self.assertFalse(args.output.exists())

    def test_protocol_rejects_truncation_reordering_bad_version_and_search_errors(self):
        protocol = self.protocol()
        broken = [protocol.rsplit("complete", 1)[0], protocol + "extra\n",
                  protocol.replace("0.4.1", "0.4.0"), protocol.replace("position\t1", "position\t0"),
                  protocol.replace("complete\t3\t3", "complete\t3\t2"),
                  "\n".join([protocol.splitlines()[0], "reject\t0\tsearch_returned_illegal_move",
                              *protocol.splitlines()[2:]]) + "\n",
                  protocol.replace("startpos moves 7g7f 3c3d", "startpos moves 7g7f 8c8d")]
        for text in broken:
            with self.subTest(text=text[:80]), self.assertRaises(ValueError):
                parse_trajectories(text, self.config())

    def test_selection_excludes_whole_trajectories_and_never_reuses_one(self):
        _, positions, _ = parse_trajectories(self.protocol(), self.config())
        duplicate = copy.deepcopy(positions[0]); duplicate["trajectory"] = 9
        selected, rejected = select_openings([positions[0], duplicate, *positions[1:]], set(), 2)
        self.assertEqual([row["trajectory"] for row in selected], [0, 1])
        self.assertEqual(rejected, [{"trajectory": 9, "reason": "duplicate_identity"}])
        selected, rejected = select_openings(positions, {positions[0]["identity_sha256"]}, 2)
        self.assertEqual([row["trajectory"] for row in selected], [1, 2])
        self.assertEqual(rejected[0]["reason"], "excluded_identity")
        with self.assertRaisesRegex(ValueError, "no corpus qualified"):
            select_openings(positions, {row["identity_sha256"] for row in positions}, 1)

    def test_complete_freeze_retains_history_and_replays_from_frozen_exclusions(self):
        with tempfile.TemporaryDirectory() as tmp:
            args, manifest = self.fixture(Path(tmp))
            evidence = validate_corpus(args.output / "manifest.json", args.output / "openings.usi")
            self.assertEqual(evidence["positions"], 2)
            self.assertEqual(evidence["openings_sha256"], manifest["openings_sha256"])
            self.assertIn("startpos moves", (args.output / "openings.usi").read_text())
            for original in args.exclusion_manifest:
                original.unlink()  # Replay uses immutable copies, not owner files.
            validate_corpus(args.output / "manifest.json", args.output / "openings.usi")
            self.assertFalse(manifest["source_ref_verified"])

    def test_mutated_or_missing_provenance_never_qualifies(self):
        mutations = [lambda m: m["artifacts"].pop("generator"),
                     lambda m: m["driver_sources"].pop("generate_search_diagnostic.py"),
                     lambda m: m.update(generation_command=["different-generator"]),
                     lambda m: m.update(validation_command=["unchecked"]),
                     lambda m: m.update(candidate_settings_used_for_generation=True)]
        for mutate in mutations:
            with tempfile.TemporaryDirectory() as tmp:
                args, manifest = self.fixture(Path(tmp))
                mutate(manifest)
                (args.output / "manifest.json").write_text(json.dumps(manifest))
                with self.assertRaises(ValueError):
                    validate_corpus(args.output / "manifest.json", args.output / "openings.usi")
        for filename in ["openings.usi", "trajectories.tsv", "generator", "match",
                         "exclusions/0-diagnostic.json", "driver-sources/generate_search_diagnostic.py"]:
            with tempfile.TemporaryDirectory() as tmp:
                args, _ = self.fixture(Path(tmp))
                with (args.output / filename).open("ab") as output:
                    output.write(b"\nchanged\n")
                with self.assertRaises((ValueError, json.JSONDecodeError)):
                    validate_corpus(args.output / "manifest.json", args.output / "openings.usi")

    def test_incomplete_marker_disqualifies_an_otherwise_complete_corpus(self):
        with tempfile.TemporaryDirectory() as tmp:
            args, _ = self.fixture(Path(tmp))
            (args.output / "incomplete.json").write_text("{}")
            with self.assertRaisesRegex(ValueError, "incomplete"):
                validate_corpus(args.output / "manifest.json", args.output / "openings.usi")

    def test_validation_failure_does_not_publish_openings_or_complete_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            args, _ = self.fixture(root)
            args.output = root / "failed"
            import subprocess
            with patch("freeze_material_gate_openings.subprocess.run", side_effect=[
                    argparse.Namespace(stdout=self.protocol(), stderr=""),
                    subprocess.CalledProcessError(2, ["validator"])]):
                with self.assertRaises(subprocess.CalledProcessError):
                    prepare(args)
            self.assertTrue((args.output / "incomplete.json").is_file())
            self.assertFalse((args.output / "openings.usi").exists())
            self.assertFalse((args.output / "manifest.json").exists())

    def test_wrapper_rejects_incomplete_corpus_before_starting_match(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            argv = ["run_ab_match.py", "selfplay", "--engine-a", "missing-a", "--engine-b", "missing-b",
                    "--openings", str(root / "missing.usi"), "--openings-manifest", str(root / "missing.json"),
                    "--out-dir", str(root / "match"), "--result-json", str(root / "result.json")]
            with patch.object(sys, "argv", argv), patch("run_ab_match.subprocess.Popen") as popen, contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(match_main(), 2)
                popen.assert_not_called()
                self.assertFalse((root / "match").exists())

    def test_wrapper_requires_material_selfplay_and_machine_readable_result(self):
        base = ["selfplay", "--engine-a", "a", "--engine-b", "b", "--openings-manifest", "manifest.json"]
        for extra in [[], ["--result-json", "result.json", "--evalfile", "weights.bin"]]:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                match_args(base + extra)


if __name__ == "__main__":
    unittest.main()
