#!/usr/bin/env python3
"""Pin the v0.4 public Rust API compatibility gates.

The standalone workflow catches a breaking ``sekirei-core`` change before
merge. The publish workflow repeats the same check on the exact release tag
before any irreversible crates.io upload. Both deliberately force a patch
compatibility check: a 0.3 -> 0.4 version bump must not silently authorize an
API break.
"""

import re
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
API_WORKFLOW = ROOT / ".github" / "workflows" / "api-compatibility.yml"
PUBLISH_WORKFLOW = ROOT / ".github" / "workflows" / "cargo-publish.yml"
ACTION = (
    "obi1kenobi/cargo-semver-checks-action@"
    "6b69fcf40e9b5fb17adeb57e4b6ecd020649a239"
)
BASELINE = "v0.3.73"


def workflow_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def semver_step(text: str, name: str) -> str:
    match = re.search(
        rf"      - name: {re.escape(name)}\n(?P<body>(?:        .+\n)+)",
        text,
    )
    if match is None:
        raise AssertionError(f"missing workflow step: {name}")
    return match.group("body")


class PublicPackageInventoryTests(unittest.TestCase):
    def test_only_published_library_is_sekirei_core(self) -> None:
        published_libraries = []
        for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
            with manifest.open("rb") as handle:
                package = tomllib.load(handle)
            if package.get("package", {}).get("publish", True) is False:
                continue
            if "lib" in package:
                published_libraries.append(package["package"]["name"])
        self.assertEqual(published_libraries, ["sekirei-core"])


class PullRequestGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = workflow_text(API_WORKFLOW)

    def test_runs_only_for_manual_or_core_relevant_changes(self) -> None:
        self.assertIn("  workflow_dispatch:\n", self.text)
        self.assertIn("  pull_request:\n    paths:\n", self.text)
        self.assertIn('      - "crates/sekirei-core/**"\n', self.text)
        self.assertNotIn('      - "crates/**"\n', self.text)

    def test_has_read_only_permissions_and_full_history(self) -> None:
        self.assertIn("permissions:\n  contents: read\n", self.text)
        self.assertIn("          fetch-depth: 0\n", self.text)

    def test_forces_patch_compatibility_against_v0373(self) -> None:
        step = semver_step(
            self.text, "Preserve the v0.3.73 public Rust API for v0.4"
        )
        self.assertIn(f"uses: {ACTION} # v2", step)
        self.assertIn("package: sekirei-core", step)
        self.assertIn(f"baseline-rev: {BASELINE}", step)
        self.assertIn("release-type: patch", step)
        self.assertIn("rust-toolchain: manual", step)


class PublishGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text = workflow_text(PUBLISH_WORKFLOW)

    def test_release_checkout_fetches_baseline_tag(self) -> None:
        checkout = self.text.index("      - uses: actions/checkout@v4")
        semver = self.text.index("      - name: Verify sekirei-core public API")
        between = self.text[checkout:semver]
        self.assertIn("          fetch-depth: 0\n", between)

    def test_same_gate_runs_before_any_publish(self) -> None:
        step = semver_step(
            self.text, "Verify sekirei-core public API compatibility"
        )
        self.assertIn(f"uses: {ACTION} # v2", step)
        self.assertIn("package: sekirei-core", step)
        self.assertIn(f"baseline-rev: {BASELINE}", step)
        self.assertIn("release-type: patch", step)
        self.assertIn("rust-toolchain: manual", step)
        self.assertIn("',sekirei-core,'", step)
        self.assertLess(
            self.text.index("Verify sekirei-core public API compatibility"),
            self.text.index("Dry-run publish sekirei-core"),
        )
        self.assertLess(
            self.text.index("Verify sekirei-core public API compatibility"),
            self.text.index("Publish sekirei-core"),
        )


if __name__ == "__main__":
    unittest.main()

