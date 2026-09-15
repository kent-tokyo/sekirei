#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("attach", ROOT / "scripts/attach_diagnostic_artifacts.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_rejects_non_diagnostic_artifact():
    artifact = ROOT / "scripts/fixtures/release_manifest_diagnostic_v1.json"
    try:
        module.attach(artifact, [artifact], ROOT / "results/floodgate/diagnostic-artifact-test.json")
    except ValueError as exc:
        assert "not diagnostic-only" in str(exc)
    else:
        raise AssertionError("non-diagnostic artifact accepted")


if __name__ == "__main__":
    test_rejects_non_diagnostic_artifact()
    print("PASS")
