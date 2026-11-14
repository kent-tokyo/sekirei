#!/usr/bin/env python3
"""Small, engine-free tests for the separate balanced-opening contract."""
import argparse
import contextlib
import copy
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import freeze_balanced_gate_openings as balanced
import freeze_material_gate_openings as original
import validate_opening_corpus as dispatcher
from generate_search_diagnostic import START, apply


class BalancedOpeningsTest(unittest.TestCase):
    def config(self):
        return {"seed": 2026111101, "trajectories": 4, "random_plies": 2,
                "min_ply": 2, "max_ply": 2, "depth": 2, "nodes": 128, "hash_mb": 1}

    def protocol(self):
        rows = ["sekirei.material-gate-trajectories.v1\t0.4.1"]
        for index, moves in enumerate(["7g7f 3c3d", "2g2f 8c8d", "7g7f 8c8d", "2g2f 3c3d"]):
            sfen = START
            for move in moves.split():
                sfen = apply(sfen, move)
            rows.append(f"position\t{index}\t2\t{sfen}\tstartpos moves {moves}")
        return "\n".join(rows + ["complete\t4\t4"]) + "\n"

    def positions(self):
        return original.parse_trajectories(self.protocol(), self.config())[1]

    def args(self, root):
        paths = []
        for role in sorted(original.ROLES):
            path = root / (role + ".json")
            path.write_text(json.dumps({"schema": "sekirei.opening-identity-exclusions.v1",
                "identity_scheme": original.IDENTITY, "role": role, "complete_for_scope": True,
                "identities": [original.identity_hash(START)],
                "sources": [{"path": "fixture-only/" + role,
                             "sha256": original.digest(role.encode())}]}))
            paths.append(path)
        generator, runner = root / "generator-input", root / "runner-input"
        generator.write_bytes(b"generator-fixture")
        runner.write_bytes(b"runner-fixture")
        return argparse.Namespace(**self.config(), count=2, timeout=10, source_ref="1" * 40,
            generator=generator, match_binary=runner, output=root / "frozen",
            exclusion_manifest=paths, max_abs_material_cp=300)

    def fixture(self, root, protocol=None):
        args = self.args(root)
        completed = [argparse.Namespace(stdout=protocol or self.protocol(), stderr=""),
                     argparse.Namespace(stdout="validated 2 openings", stderr="")]
        with patch.object(balanced.subprocess, "run", side_effect=completed) as run:
            manifest = balanced.prepare(args)
            self.assertEqual(run.call_count, 2)
        return args, manifest

    def save_manifest(self, args, manifest):
        (args.output / "manifest.json").write_text(json.dumps(manifest))

    def validate(self, args):
        return balanced.validate_corpus(args.output / "manifest.json", args.output / "openings.usi")

    def test_material_threshold_is_inclusive_for_both_sides(self):
        # The one-cp boundary also guards future changes to the material scale.
        positions = self.positions()
        before = copy.deepcopy(positions)
        with patch.object(balanced, "material", side_effect=[300, -300, 301, -301]):
            selected, rejected = balanced.select_openings(positions, set(), 4, 300)
        self.assertEqual([row["trajectory"] for row in selected], [0, 1])
        self.assertEqual([row["material_cp"] for row in selected], [300, -300])
        self.assertEqual([(row["trajectory"], row["reason"], row["material_cp"])
                          for row in rejected], [(2, "material_limit", 301), (3, "material_limit", -301)])
        self.assertEqual(positions, before)

    def test_material_filter_counts_hands_and_promotions_independent_of_side_to_move(self):
        empty = "9/9/9/9/9/9/9/9/9"
        cases = [(empty + " b 3P 1", 300), (empty + " w 3p 1", -300),
                 ("4+P4/9/9/9/9/9/9/9/9 b 3p 1", 300),
                 ("4+r4/9/9/9/9/9/9/9/9 w R 1", -260),
                 (empty + " b 4P 1", 400)]
        positions = [{"trajectory": index, "sfen": sfen,
                      "identity_sha256": original.identity_hash(sfen)}
                     for index, (sfen, _) in enumerate(cases)]
        selected, rejected = balanced.select_openings(positions, set(), 5, 300)
        self.assertEqual([row["material_cp"] for row in selected], [score for _, score in cases[:-1]])
        self.assertEqual([(row["trajectory"], row["reason"], row["material_cp"])
                          for row in rejected], [(4, "material_limit", 400)])

    def test_selection_preserves_order_exclusion_duplicate_and_after_cap_journal(self):
        positions = self.positions()
        duplicate = {**positions[1], "trajectory": 9}
        sequence = [positions[0], positions[1], duplicate, positions[2], positions[3]]
        selected, rejected = balanced.select_openings(sequence, {positions[0]["identity_sha256"]}, 2, 300)
        self.assertEqual([row["trajectory"] for row in selected], [1, 2])
        self.assertEqual([(row["trajectory"], row["reason"]) for row in rejected],
                         [(0, "excluded_identity"), (9, "duplicate_identity"), (3, "after_requested_count")])
        self.assertEqual(len(selected) + len(rejected), len(sequence))

    def test_invalid_policy_prevents_allocation_and_generation(self):
        for value in [-1, True, 300.0, 100001]:
            with self.subTest(value=value), tempfile.TemporaryDirectory() as tmp:
                args = self.args(Path(tmp))
                args.max_abs_material_cp = value
                with patch.object(balanced.subprocess, "run") as run, self.assertRaises(ValueError):
                    balanced.prepare(args)
                run.assert_not_called()
                self.assertFalse(args.output.exists())

    def test_cli_requires_explicit_material_limit(self):
        argv = ["freeze_balanced_gate_openings.py", "--generator", "g", "--match-binary", "m",
                "--output", "out", "--source-ref", "revision", "--exclusion-manifest", "excluded.json",
                "--seed", "1"]
        with patch.object(sys, "argv", argv), patch.object(balanced.subprocess, "run") as run:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as stopped:
                balanced.main()
            self.assertEqual(stopped.exception.code, 2)
            run.assert_not_called()

    def test_complete_freeze_keeps_sources_plan_full_history_and_full_journal(self):
        with tempfile.TemporaryDirectory() as tmp:
            args, manifest = self.fixture(Path(tmp))
            evidence = self.validate(args)
            self.assertEqual(dispatcher.validate_corpus(args.output / "manifest.json", args.output / "openings.usi"), evidence)
            self.assertEqual(evidence["schema"], "sekirei.independent-balanced-material-openings.v1")
            self.assertEqual(evidence["positions"], 2)
            self.assertEqual(set(manifest["driver_sources"]), {
                "freeze_material_gate_openings.py", "generate_search_diagnostic.py",
                "paired_search_diagnostics.py", "nps_threads.py", "freeze_balanced_gate_openings.py",
                "analyze_paired_openings.py", "stream_opening_exclusions.py"})
            self.assertEqual(manifest["selection_plan_sha256"],
                             original.digest((args.output / "selection-plan.json").read_bytes()))
            self.assertEqual((args.output / "openings.usi").read_text().splitlines(),
                             [row["position"] for row in self.positions()[:2]])
            self.assertEqual([row["trajectory"] for row in manifest["selection_rejections"]], [2, 3])
            self.assertEqual({row["reason"] for row in manifest["selection_rejections"]}, {"after_requested_count"})
            self.assertEqual(json.loads((args.output / "selection.json").read_text()),
                             {key: manifest[key] for key in ("positions", "selection_rejections", "terminal_rejections")})
            for path in args.exclusion_manifest:
                path.unlink()
            self.validate(args)  # Only the frozen exclusion copies are needed.
            with self.assertRaises(ValueError):
                original.validate_corpus(args.output / "manifest.json", args.output / "openings.usi")

    def test_plan_exists_before_generator_and_validator_are_called(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = self.args(Path(tmp))
            calls = []
            def run(command, **kwargs):
                plan = json.loads((args.output / "selection-plan.json").read_text())
                self.assertEqual(plan["status"], "planned")
                self.assertEqual(plan["material_policy"]["max_abs_material_cp"], 300)
                self.assertEqual(plan["requested_count"], 2)
                calls.append(command)
                return argparse.Namespace(stdout=self.protocol() if len(calls) == 1 else "validated", stderr="")
            with patch.object(balanced.subprocess, "run", side_effect=run):
                balanced.prepare(args)
            self.assertEqual(len(calls), 2)

    def test_terminal_rejections_remain_separate_from_selection_rejections(self):
        rows = self.protocol().splitlines()
        rows[-2] = "reject\t3\tterminal_before_sample"
        rows[-1] = "complete\t4\t3"
        with tempfile.TemporaryDirectory() as tmp:
            args, manifest = self.fixture(Path(tmp), "\n".join(rows) + "\n")
            self.assertEqual(manifest["terminal_rejections"], [{"trajectory": 3, "reason": "terminal_before_sample"}])
            self.assertEqual([row["trajectory"] for row in manifest["selection_rejections"]], [2])
            self.validate(args)

    def test_exhaustion_retains_partial_journal_without_qualified_corpus(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = self.args(Path(tmp))
            path = args.exclusion_manifest[0]
            exclusion = json.loads(path.read_text())
            exclusion["identities"].extend(row["identity_sha256"] for row in self.positions()[1:])
            path.write_text(json.dumps(exclusion))
            with patch.object(balanced.subprocess, "run", return_value=argparse.Namespace(
                    stdout=self.protocol(), stderr="")) as run, self.assertRaisesRegex(ValueError, "no corpus qualified"):
                balanced.prepare(args)
            self.assertEqual(run.call_count, 1)
            journal = json.loads((args.output / "selection.json").read_text())
            self.assertEqual([row["trajectory"] for row in journal["positions"]], [0])
            self.assertEqual([row["trajectory"] for row in journal["selection_rejections"]], [1, 2, 3])
            self.assertEqual((args.output / "trajectories.tsv").read_text(), self.protocol())
            self.assertTrue((args.output / "selection-plan.json").is_file())
            self.assertTrue((args.output / "incomplete.json").is_file())
            for filename in ["manifest.json", "openings.usi", "openings.pending"]:
                self.assertFalse((args.output / filename).exists())

    def test_manifest_policy_selection_and_provenance_tampering_is_rejected(self):
        mutations = [lambda m: m["material_policy"].update(max_abs_material_cp=400),
                     lambda m: m["material_policy"]["values"].update(P=101),
                     lambda m: m["material_policy"].update(direction="side-to-move"),
                     lambda m: m.update(selection="lowest candidate validation loss"),
                     lambda m: m["positions"][0].update(material_cp=301),
                     lambda m: m["positions"].pop(),
                     lambda m: m["selection_rejections"].pop(),
                     lambda m: m.update(candidate_settings_used_for_generation=True),
                     lambda m: m.update(strength_claim=True),
                     lambda m: m["driver_sources"].pop("stream_opening_exclusions.py"),
                     lambda m: m["artifacts"].pop("generator")]
        for index, mutate in enumerate(mutations):
            with self.subTest(mutation=index), tempfile.TemporaryDirectory() as tmp:
                args, manifest = self.fixture(Path(tmp))
                mutate(manifest)
                self.save_manifest(args, manifest)
                with self.assertRaises(ValueError):
                    self.validate(args)

    def test_frozen_file_tampering_is_rejected(self):
        filenames = ["openings.usi", "trajectories.tsv", "selection-plan.json", "selection.json",
                     "generator", "match", "exclusions/0-diagnostic.json",
                     "driver-sources/analyze_paired_openings.py", "driver-sources/stream_opening_exclusions.py"]
        for filename in filenames:
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as tmp:
                args, _ = self.fixture(Path(tmp))
                with (args.output / filename).open("ab") as output:
                    output.write(b"\nchanged\n")
                with self.assertRaises(ValueError):
                    self.validate(args)

    def test_rehashed_raw_header_or_history_tampering_still_fails_replay(self):
        mutations = [lambda raw: raw.replace("0.4.1", "0.4.0"),
                     lambda raw: raw.replace("startpos moves 7g7f 3c3d", "startpos moves 7g7f 8c8d")]
        for index, mutate in enumerate(mutations):
            with self.subTest(mutation=index), tempfile.TemporaryDirectory() as tmp:
                args, manifest = self.fixture(Path(tmp))
                path = args.output / "trajectories.tsv"
                path.write_text(mutate(path.read_text()))
                manifest["raw_trajectories_sha256"] = original.digest(path.read_bytes())
                self.save_manifest(args, manifest)
                with self.assertRaises(ValueError):
                    self.validate(args)

    def test_rehashed_corpus_cannot_drop_history_or_reorder_selected_openings(self):
        for mutate in [lambda rows: ["sfen " + self.positions()[0]["sfen"], rows[1]], lambda rows: rows[::-1]]:
            with tempfile.TemporaryDirectory() as tmp:
                args, manifest = self.fixture(Path(tmp))
                path = args.output / "openings.usi"
                path.write_text("\n".join(mutate(path.read_text().splitlines())) + "\n")
                manifest["openings_sha256"] = original.digest(path.read_bytes())
                self.save_manifest(args, manifest)
                with self.assertRaises(ValueError):
                    self.validate(args)

    def test_replay_uses_exclusion_identities_not_just_record_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            args, manifest = self.fixture(Path(tmp))
            record = manifest["exclusions"][0]
            path = Path(record["path"])
            exclusions = json.loads(path.read_text())
            exclusions["identities"].append(manifest["positions"][0]["identity_sha256"])
            path.write_text(json.dumps(exclusions))
            record.update(sha256=original.digest(path.read_bytes()), count=record["count"] + 1)
            # Update the recipe pins too, so selection replay must catch the conflict.
            plan_path = args.output / "selection-plan.json"
            plan = json.loads(plan_path.read_text())
            plan["exclusions"] = manifest["exclusions"]
            plan_path.write_text(json.dumps(plan))
            manifest["selection_plan_sha256"] = original.digest(plan_path.read_bytes())
            self.save_manifest(args, manifest)
            with self.assertRaisesRegex(ValueError, "replay"):
                self.validate(args)

    def test_failed_validation_preserves_attempt_without_publishing(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = self.args(Path(tmp))
            with patch.object(balanced.subprocess, "run", side_effect=[
                    argparse.Namespace(stdout=self.protocol(), stderr=""),
                    subprocess.CalledProcessError(2, ["validator"])]), self.assertRaises(subprocess.CalledProcessError):
                balanced.prepare(args)
            self.assertTrue((args.output / "selection.json").is_file())
            self.assertTrue((args.output / "incomplete.json").is_file())
            self.assertFalse((args.output / "manifest.json").exists())
            self.assertFalse((args.output / "openings.usi").exists())

    def test_mutation_during_validation_cannot_publish_complete_corpus(self):
        for filename in ["trajectories.tsv", "selection.json"]:
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as tmp:
                args = self.args(Path(tmp))
                def run(command, **kwargs):
                    if "validate-positions" in command:
                        path = args.output / filename
                        if filename == "selection.json":
                            journal = json.loads(path.read_text())
                            journal["positions"].pop()
                            path.write_text(json.dumps(journal))
                        else:
                            path.write_text(path.read_text() + "changed\n")
                        return argparse.Namespace(stdout="validated", stderr="")
                    return argparse.Namespace(stdout=self.protocol(), stderr="")
                with patch.object(balanced.subprocess, "run", side_effect=run), self.assertRaises(ValueError):
                    balanced.prepare(args)
                self.assertTrue((args.output / "incomplete.json").is_file())
                self.assertFalse((args.output / "manifest.json").exists())
                self.assertFalse((args.output / "openings.usi").exists())

    def test_import_from_unpinned_source_prevents_generation(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = self.args(Path(tmp))
            with patch.object(sys.modules["analyze_paired_openings"], "__file__", str(Path(tmp) / "foreign.py")):
                with patch.object(balanced.subprocess, "run") as run, self.assertRaisesRegex(ValueError, "imported driver"):
                    balanced.prepare(args)
                run.assert_not_called()
            self.assertTrue((args.output / "incomplete.json").is_file())
            self.assertFalse((args.output / "manifest.json").exists())

    def test_incomplete_marker_disqualifies_even_complete_corpus(self):
        with tempfile.TemporaryDirectory() as tmp:
            args, _ = self.fixture(Path(tmp))
            (args.output / "incomplete.json").write_text("{}")
            with self.assertRaisesRegex(ValueError, "incomplete"):
                self.validate(args)


if __name__ == "__main__":
    unittest.main()
