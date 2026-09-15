import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "validator", ROOT / "scripts/validate_floodgate_candidate_evidence.py"
)
validator = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(validator)


def test_current_decision_references_valid_artifacts():
    document = json.loads(
        (ROOT / "results/floodgate/20260912-review/fg5e-candidate-decision.json").read_text()
    )
    assert validator.validate(document, ROOT) == []


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
    document = json.loads(
        (ROOT / "results/floodgate/20260912-review/fg5e-candidate-decision.json").read_text()
    )
    document["evidence"]["resource_preflight"] = "pass"
    assert "evidence.resource_preflight" in validator.validate(document, ROOT)


if __name__ == "__main__":
    test_current_decision_references_valid_artifacts()
    print("PASS")
