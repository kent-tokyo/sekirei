#!/usr/bin/env python3
"""Structural and behavioral tests for .github/workflows/cargo-publish.yml.

Motivation: a crates.io publish can never be fully undone (a bad version
can be yanked, never deleted), so this workflow's safety properties --
which git ref actually gets published, which steps can see the registry
token, which crates are accepted, and in what order publishing happens -- are
pinned here rather than trusted to hold by inspection alone.
Written after a review round caught: the original workflow published
whatever ref triggered workflow_dispatch (not a specific release tag), put
CARGO_REGISTRY_TOKEN in job-level env (visible to every step, not just the
ones that upload), and would happily start publishing sekirei-core/bench/
csa/match-runner before discovering sekirei-train/sekirei could not actually
be published partway through an irreversible sequence. A second round then
replaced the long-lived
CARGO_REGISTRY_TOKEN repo secret entirely with crates.io Trusted Publishing
(OIDC via rust-lang/crates-io-auth-action) -- sekirei-core/bench/csa/
match-runner already have prior published versions on crates.io, so the
"first release must use a token" constraint in crates.io's own Trusted
Publishing rules doesn't block this. TokenScopeTests pins that no
CARGO_REGISTRY_TOKEN secret reference exists anywhere in the file at all,
not just that it's been moved somewhere narrower.

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

    def test_permissions_are_exactly_contents_read_and_id_token_write(self):
        # Exact-match, not subset, so an unrelated permission can't creep in
        # unnoticed later -- id-token: write is required for Trusted
        # Publishing (see TokenScopeTests.test_id_token_write_permission_present).
        self.assertEqual(_load()["permissions"], {"contents": "read", "id-token": "write"})

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


CRATE_AUTH_STEPS = [
    ("sekirei-core", "Authenticate with crates.io (sekirei-core)", "auth-core", "Publish sekirei-core"),
    ("sekirei-bench", "Authenticate with crates.io (sekirei-bench)", "auth-bench", "Publish sekirei-bench"),
    ("sekirei-csa", "Authenticate with crates.io (sekirei-csa)", "auth-csa", "Publish sekirei-csa"),
    (
        "sekirei-match-runner",
        "Authenticate with crates.io (sekirei-match-runner)",
        "auth-match-runner",
        "Publish sekirei-match-runner",
    ),
    ("sekirei-train", "Authenticate with crates.io (sekirei-train)", "auth-train", "Publish sekirei-train"),
    ("sekirei", "Authenticate with crates.io (sekirei)", "auth-usi", "Publish sekirei (sekirei-usi)"),
]

DRY_RUN_STEP_NAMES = [
    "Dry-run publish sekirei-core",
    "Dry-run publish sekirei-bench",
    "Dry-run publish sekirei-csa",
    "Dry-run publish sekirei-match-runner",
    "Dry-run publish sekirei-train",
    "Dry-run publish sekirei (sekirei-usi)",
]


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed -- pip install pyyaml")
class TokenScopeTests(unittest.TestCase):
    """No CARGO_REGISTRY_TOKEN secret exists anywhere in this repo for this
    workflow -- every real publish step gets a short-lived OIDC token from
    its own preceding Trusted Publishing auth step instead (rust-lang/
    crates-io-auth-action). These tests pin that there is no long-lived
    token fallback anywhere, not just that the OIDC path exists."""

    def test_no_secrets_reference_anywhere_in_file(self):
        text = WORKFLOW_PATH.read_text()
        self.assertNotIn("secrets.CARGO_REGISTRY_TOKEN", text)
        self.assertNotIn("secrets.", text, "no step in this workflow should reference any repo secret")

    def test_id_token_write_permission_present(self):
        self.assertEqual(_load()["permissions"].get("id-token"), "write")

    def test_no_workflow_level_env(self):
        self.assertNotIn("env", _load())

    def test_no_job_level_token(self):
        job_env = _load()["jobs"]["publish"].get("env", {})
        self.assertNotIn("CARGO_REGISTRY_TOKEN", job_env)

    def test_each_crate_has_a_dedicated_auth_step(self):
        for crate, auth_name, auth_id, _publish_name in CRATE_AUTH_STEPS:
            with self.subTest(crate=crate):
                auth_step = _step_by_name(auth_name)
                self.assertEqual(auth_step.get("id"), auth_id)
                self.assertTrue(
                    auth_step.get("uses", "").startswith("rust-lang/crates-io-auth-action@"),
                    f"{auth_name} must use rust-lang/crates-io-auth-action, got {auth_step.get('uses')!r}",
                )

    def test_each_publish_step_uses_its_own_auth_step_output(self):
        for crate, _auth_name, auth_id, publish_name in CRATE_AUTH_STEPS:
            with self.subTest(crate=crate):
                publish_step = _step_by_name(publish_name)
                token_expr = publish_step.get("env", {}).get("CARGO_REGISTRY_TOKEN")
                self.assertEqual(token_expr, f"${{{{ steps.{auth_id}.outputs.token }}}}")

    def test_auth_step_and_publish_step_share_the_same_if_condition(self):
        for crate, auth_name, _auth_id, publish_name in CRATE_AUTH_STEPS:
            with self.subTest(crate=crate):
                auth_if = _step_by_name(auth_name).get("if")
                publish_if = _step_by_name(publish_name).get("if")
                self.assertEqual(auth_if, publish_if)

    def test_dry_run_steps_never_authenticate_or_see_a_token(self):
        for name in DRY_RUN_STEP_NAMES:
            with self.subTest(step=name):
                step = _step_by_name(name)
                self.assertNotIn("CARGO_REGISTRY_TOKEN", step.get("env", {}))
                self.assertNotIn("crates-io-auth-action", step.get("uses", ""))

    def test_no_step_outside_the_known_auth_steps_uses_the_auth_action(self):
        known_auth_names = {auth_name for _c, auth_name, _id, _p in CRATE_AUTH_STEPS}
        for s in _steps():
            if "crates-io-auth-action" in s.get("uses", ""):
                self.assertIn(s.get("name"), known_auth_names)


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

    def test_each_auth_step_immediately_precedes_its_publish_step(self):
        # GitHub Actions runs steps in file order -- if "Publish X" appeared
        # before "Authenticate with crates.io (X)", steps.auth-X.outputs.token
        # would be empty when the publish step ran.
        for crate, auth_name, _auth_id, publish_name in CRATE_AUTH_STEPS:
            with self.subTest(crate=crate):
                auth_idx = _step_index_by_name(auth_name)
                publish_idx = _step_index_by_name(publish_name)
                self.assertEqual(publish_idx, auth_idx + 1)

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
class RegistryCrateBehaviorTests(unittest.TestCase):
    def test_no_stale_lineprior_blocker_remains(self):
        names = [s.get("name", "") for s in _steps()]
        self.assertFalse(any("blocked" in name.lower() for name in names))

    def test_all_workspace_crates_have_publish_steps(self):
        names = {s.get("name", "") for s in _steps()}
        expected = {
            "Publish sekirei-core",
            "Publish sekirei-bench",
            "Publish sekirei-csa",
            "Publish sekirei-match-runner",
            "Publish sekirei-train",
            "Publish sekirei (sekirei-usi)",
        }
        self.assertTrue(expected <= names)


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

    def test_all_workspace_packages_match_release_tag(self):
        current = _current_sekirei_core_version()
        result = self._run(f"v{current}", ",".join(ALL_CRATE_NAMES))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("All workspace crates match", result.stdout)

    def test_mismatched_version_refused(self):
        result = self._run("v0.0.1", "sekirei-core")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("VERSION_TAG_MISMATCH", result.stdout)

    def test_version_check_includes_cargo_lock(self):
        run_script = _step_by_name("Verify Cargo.toml version matches release_tag")["run"]
        self.assertIn('open("Cargo.lock", "rb")', run_script)
        self.assertIn("lock_mismatches", run_script)


if __name__ == "__main__":
    unittest.main()
