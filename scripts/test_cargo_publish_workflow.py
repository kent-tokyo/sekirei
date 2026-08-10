#!/usr/bin/env python3
"""Structural and behavioral tests for .github/workflows/cargo-publish.yml.

Motivation: a crates.io publish can never be fully undone (a bad version
can be yanked, never deleted), so this workflow's safety properties --
which git ref actually gets published, which steps can see the registry
token, which crates are refused outright, and in what order publishing
happens -- are pinned here rather than trusted to hold by inspection alone.
Written after a review round caught: the original workflow published
whatever ref triggered workflow_dispatch (not a specific release tag), put
CARGO_REGISTRY_TOKEN in job-level env (visible to every step, not just the
ones that upload), and would happily start publishing sekirei-core/bench/
csa/match-runner before discovering sekirei-train/sekirei can't actually be
published (lineprior is git-pinned, not on crates.io) partway through an
irreversible sequence.

Structural tests parse the real YAML (requires PyYAML; skipped with a
clear message if unavailable -- this test file is a local verification
tool, same convention as scripts/test_run_fixed_depth_ab.py, not wired
into CI). Behavioral tests extract the embedded preflight Python from each
relevant `run:` block and execute it as a real subprocess against
controlled environment variables -- the same script bytes GitHub Actions
would run, not a reimplementation that could silently drift from the
workflow file.

Run: python3 scripts/test_cargo_publish_workflow.py -v
"""
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

try:
    import yaml

    HAVE_YAML = True
except ImportError:
    HAVE_YAML = False

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "cargo-publish.yml"

ALL_CRATE_NAMES = [
    "sekirei-core",
    "sekirei-bench",
    "sekirei-csa",
    "sekirei-match-runner",
    "sekirei-train",
    "sekirei",
]
BLOCKED_CRATE_NAMES = ["sekirei-train", "sekirei"]  # lineprior git dependency


def _load():
    with open(WORKFLOW_PATH) as f:
        return yaml.safe_load(f)


def _on_triggers(wf):
    # PyYAML (YAML 1.1) parses the bare scalar key `on:` as boolean True,
    # not the string "on" -- this is a parser quirk, not a bug in the file
    # (GitHub's own workflow parser reads it as the literal key "on").
    return wf[True] if True in wf else wf["on"]


def _steps():
    return _load()["jobs"]["publish"]["steps"]


def _step_by_name(name):
    for s in _steps():
        if s.get("name") == name:
            return s
    raise KeyError(f"no step named {name!r}")


def _step_index_by_name(name):
    for i, s in enumerate(_steps()):
        if s.get("name") == name:
            return i
    raise KeyError(f"no step named {name!r}")


def _step_by_uses_prefix(prefix):
    for s in _steps():
        if s.get("uses", "").startswith(prefix):
            return s
    raise KeyError(f"no step using {prefix!r}")


def _extract_python_heredoc(run_script):
    marker = "<<'PYEOF'"
    start = run_script.index(marker) + len(marker)
    end = run_script.index("PYEOF", start)
    return run_script[start:end]


def _run_embedded_python(step_name, env, cwd=None):
    py = _extract_python_heredoc(_step_by_name(step_name)["run"])
    with tempfile.NamedTemporaryFile("w", suffix=".py", delete=False) as f:
        f.write(py)
        path = f.name
    return subprocess.run(
        [sys.executable, path], capture_output=True, text=True, env=env, cwd=cwd
    )


def _current_sekirei_core_version():
    with open(REPO_ROOT / "crates" / "sekirei-core" / "Cargo.toml", "rb") as f:
        return tomllib.load(f)["package"]["version"]


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed -- pip install pyyaml")
class TriggerAndPermissionsTests(unittest.TestCase):
    def test_workflow_dispatch_only(self):
        self.assertEqual(list(_on_triggers(_load()).keys()), ["workflow_dispatch"])

    def test_permissions_contents_read_only(self):
        self.assertEqual(_load()["permissions"], {"contents": "read"})

    def test_dry_run_input_defaults_true(self):
        inputs = _on_triggers(_load())["workflow_dispatch"]["inputs"]
        self.assertTrue(inputs["dry_run"]["default"])
        self.assertTrue(inputs["dry_run"]["required"])

    def test_release_tag_input_required_no_hidden_default_bypass(self):
        inputs = _on_triggers(_load())["workflow_dispatch"]["inputs"]
        self.assertTrue(inputs["release_tag"]["required"])


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed -- pip install pyyaml")
class CheckoutPinnedToReleaseTagTests(unittest.TestCase):
    def test_checkout_ref_is_release_tag_input(self):
        checkout = _step_by_uses_prefix("actions/checkout@")
        self.assertEqual(checkout["with"]["ref"], "refs/tags/${{ inputs.release_tag }}")


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed -- pip install pyyaml")
class TokenScopeTests(unittest.TestCase):
    def test_no_workflow_level_token(self):
        self.assertNotIn("env", _load())

    def test_no_job_level_token(self):
        job_env = _load()["jobs"]["publish"].get("env", {})
        self.assertNotIn("CARGO_REGISTRY_TOKEN", job_env)

    def test_only_real_publish_steps_have_token(self):
        for crate_display, step_name in [
            ("sekirei-core", "Publish sekirei-core"),
            ("sekirei-bench", "Publish sekirei-bench"),
            ("sekirei-csa", "Publish sekirei-csa"),
            ("sekirei-match-runner", "Publish sekirei-match-runner"),
            ("sekirei-train", "Publish sekirei-train"),
            ("sekirei", "Publish sekirei (sekirei-usi)"),
        ]:
            with self.subTest(crate=crate_display):
                real_step = _step_by_name(step_name)
                self.assertIn("CARGO_REGISTRY_TOKEN", real_step.get("env", {}))

        for crate_display, step_name in [
            ("sekirei-core", "Dry-run publish sekirei-core"),
            ("sekirei-bench", "Dry-run publish sekirei-bench"),
            ("sekirei-csa", "Dry-run publish sekirei-csa"),
            ("sekirei-match-runner", "Dry-run publish sekirei-match-runner"),
            ("sekirei-train", "Dry-run publish sekirei-train"),
            ("sekirei", "Dry-run publish sekirei (sekirei-usi)"),
        ]:
            with self.subTest(crate=crate_display):
                dry_step = _step_by_name(step_name)
                self.assertNotIn("CARGO_REGISTRY_TOKEN", dry_step.get("env", {}))

    def test_no_other_step_has_token(self):
        exempt = {f"Publish {c}" for c in ALL_CRATE_NAMES} | {"Publish sekirei (sekirei-usi)"}
        for s in _steps():
            name = s.get("name", s.get("uses", "?"))
            if name in exempt:
                continue
            self.assertNotIn(
                "CARGO_REGISTRY_TOKEN", s.get("env", {}), f"unexpected token exposure in step: {name}"
            )


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed -- pip install pyyaml")
class PublishOrderTests(unittest.TestCase):
    def test_core_publishes_before_wait_before_dependents(self):
        core_idx = _step_index_by_name("Publish sekirei-core")
        wait_idx = _step_index_by_name("Wait for sekirei-core to index on crates.io")
        self.assertLess(core_idx, wait_idx)
        for name in [
            "Publish sekirei-bench",
            "Publish sekirei-csa",
            "Publish sekirei-match-runner",
            "Publish sekirei-train",
            "Publish sekirei (sekirei-usi)",
        ]:
            with self.subTest(step=name):
                self.assertGreater(_step_index_by_name(name), wait_idx)

    def test_blocked_crate_guard_runs_before_any_publish_step(self):
        guard_idx = _step_index_by_name("Reject known-blocked crates (lineprior not on crates.io)")
        for s in _steps():
            name = s.get("name", "")
            if name.startswith("Publish ") or name.startswith("Dry-run publish "):
                self.assertLess(
                    guard_idx, _steps().index(s), f"blocked-crate guard must precede {name!r}"
                )


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed -- pip install pyyaml")
class ValidateCratesInputBehaviorTests(unittest.TestCase):
    def _run(self, crates_input):
        return _run_embedded_python("Validate crates input", env={"CRATES_INPUT": crates_input})

    def test_single_known_crate_accepted(self):
        result = self._run("sekirei-core")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_multiple_crates_with_whitespace_accepted(self):
        result = self._run("sekirei-core, sekirei-bench")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_unknown_crate_refused(self):
        result = self._run("sekirei-core,not-a-crate")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown crate", result.stdout)

    def test_duplicate_crate_refused(self):
        result = self._run("sekirei-core,sekirei-core")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("duplicate", result.stdout)

    def test_empty_input_refused(self):
        result = self._run("")
        self.assertNotEqual(result.returncode, 0)

    def test_trailing_comma_refused(self):
        # A shell `read -ra ... <<<` silently drops a trailing empty field
        # at EOF -- this is exactly the silent-skip failure mode flagged in
        # review; pin that the Python implementation does NOT reproduce it.
        result = self._run("sekirei-core,")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("empty entry", result.stdout)

    def test_doubled_comma_refused(self):
        result = self._run("sekirei-core,,sekirei-bench")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("empty entry", result.stdout)


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed -- pip install pyyaml")
class BlockedCrateBehaviorTests(unittest.TestCase):
    def _run(self, crates_input):
        return _run_embedded_python(
            "Reject known-blocked crates (lineprior not on crates.io)",
            env={"CRATES_INPUT": crates_input},
        )

    def test_unblocked_crates_pass(self):
        result = self._run("sekirei-core,sekirei-bench,sekirei-csa,sekirei-match-runner")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_sekirei_train_refused(self):
        result = self._run("sekirei-train")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("BLOCKED_REGISTRY_DEPENDENCY", result.stdout)

    def test_sekirei_usi_package_refused(self):
        result = self._run("sekirei")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("BLOCKED_REGISTRY_DEPENDENCY", result.stdout)

    def test_blocked_crate_mixed_with_allowed_still_refused(self):
        result = self._run("sekirei-core,sekirei-train")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("BLOCKED_REGISTRY_DEPENDENCY", result.stdout)


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed -- pip install pyyaml")
class VersionTagMatchBehaviorTests(unittest.TestCase):
    def _run(self, release_tag, crates_input):
        return _run_embedded_python(
            "Verify Cargo.toml version matches release_tag",
            env={"RELEASE_TAG": release_tag, "CRATES_INPUT": crates_input},
            cwd=str(REPO_ROOT),
        )

    def test_matching_version_passes(self):
        current = _current_sekirei_core_version()
        result = self._run(f"v{current}", "sekirei-core")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_mismatched_version_refused(self):
        result = self._run("v0.0.1", "sekirei-core")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("VERSION_TAG_MISMATCH", result.stdout)


if __name__ == "__main__":
    unittest.main()
