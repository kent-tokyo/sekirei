import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "validator", ROOT / "scripts/validate_floodgate_candidate_evidence.py"
)
validator = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(validator)


def write_valid_evidence(root: Path) -> dict[str, object]:
    decision = {
        "schema": "sekirei.floodgate-candidate-decision.v1",
        "diagnostic_only": True,
        "status": "inconclusive",
        "decision": "keep_current",
        "candidate": {"version": "0.3.36", "formal_adoption": False},
        "evidence": {
            "pilot_manifest": "pilot.json",
            "holdout_manifest": "holdout.json",
            "candidate_manifest": "candidate.json",
            "holdout_regression_artifact": "regression.json",
            "resource_preflight_artifact": "preflight.json",
            "holdout_alignment_artifact": "alignment.json",
            "gate_status": "ready",
            "version_alignment": "historical_evidence_not_candidate",
            "holdout_regression": "clean_for_this_diagnostic",
            "nmp_ablation": "none",
            "resource_preflight": "refuse",
        },
        "reasons": ["test fixture remains diagnostic-only"],
        "next_action": "run an independent strength gate",
        "claims": {
            "strength": "not_permitted",
            "release_approval": "not_granted",
            "competitor_superiority": "not_established",
        },
    }
    for name in ("pilot.json", "holdout.json"):
        (root / name).write_text(json.dumps({"schema": "sekirei.release-manifest.v1"}))
    (root / "candidate.json").write_text(json.dumps({
        "schema": "sekirei.floodgate-review-manifest.v1",
        "provenance": {"source_revision": "fixture"},
        "run_contract": {"engine_version": "0.3.36"},
    }))
    (root / "regression.json").write_text(json.dumps({
        "schema": "sekirei.floodgate-holdout-regression.v1",
        "status": "clean_for_this_diagnostic",
        "diagnostic_only": True,
    }))
    (root / "preflight.json").write_text(json.dumps({
        "schema": "sekirei.gate-resource-preflight.v1", "verdict": "refuse",
    }))
    (root / "alignment.json").write_text(json.dumps({
        "schema": "sekirei.floodgate-candidate-holdout-alignment.v1",
        "status": "not_aligned",
        "reasons": ["holdout_release_version_mismatch"],
    }))
    return decision


def test_current_decision_references_valid_artifacts():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        assert validator.validate(write_valid_evidence(root), root) == []


def test_rejects_non_clean_regression(tmp_path):
    decision = {
        "schema": "sekirei.floodgate-candidate-decision.v1", "diagnostic_only": True,
        "status": "inconclusive", "decision": "keep_current",
        "candidate": {"version": "0.3.36", "formal_adoption": False},
        "evidence": {
            "pilot_manifest": "pilot.json", "holdout_manifest": "holdout.json",
            "candidate_manifest": "candidate.json",
            "holdout_regression_artifact": "regression.json", "resource_preflight_artifact": "preflight.json",
            "holdout_alignment_artifact": "alignment.json",
            "gate_status": "ready", "version_alignment": "historical_evidence_not_candidate",
            "holdout_regression": "clean", "nmp_ablation": "none", "resource_preflight": "refuse",
        },
        "reasons": ["test"], "next_action": "test",
        "claims": {"strength": "not_permitted", "release_approval": "not_granted",
                   "competitor_superiority": "not_established"},
    }
    for name in ("pilot.json", "holdout.json"):
        (tmp_path / name).write_text(json.dumps({"schema": "sekirei.release-manifest.v1"}))
    (tmp_path / "candidate.json").write_text(json.dumps({
        "schema": "sekirei.floodgate-review-manifest.v1",
        "provenance": {"source_revision": "working-tree-uncommitted"},
        "run_contract": {"engine_version": "0.3.36"},
    }))
    (tmp_path / "regression.json").write_text(json.dumps({
        "schema": "sekirei.floodgate-holdout-regression.v1",
        "status": "inconclusive", "diagnostic_only": True,
    }))
    (tmp_path / "preflight.json").write_text(json.dumps({
        "schema": "sekirei.gate-resource-preflight.v1", "verdict": "refuse",
    }))
    (tmp_path / "alignment.json").write_text(json.dumps({
        "schema": "sekirei.floodgate-candidate-holdout-alignment.v1",
        "status": "not_aligned", "reasons": ["holdout_release_version_mismatch"],
    }))
    assert "evidence.holdout_regression_artifact.status" in validator.validate(decision, tmp_path)


def test_rejects_stale_status_summary(tmp_path):
    document = write_valid_evidence(tmp_path)
    document["evidence"]["resource_preflight"] = "pass"
    assert "evidence.resource_preflight" in validator.validate(document, tmp_path)


if __name__ == "__main__":
    test_current_decision_references_valid_artifacts()
    print("PASS")
