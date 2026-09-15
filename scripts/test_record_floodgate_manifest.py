#!/usr/bin/env python3
import importlib.util
import json
from unittest.mock import patch
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("record", ROOT / "scripts/record_floodgate_manifest.py")
record = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(record)


def test_fixture_pair_has_integrity_record():
    csa = ROOT / "scripts/fixtures/analysis_replay_v1.csa"
    sidecar = ROOT / "scripts/fixtures/analysis_replay_v1.analysis.jsonl"
    pair = record.pair_record(csa, sidecar, ROOT)
    assert pair["status"] == "verified"
    assert pair["csa"]["moves"] == 1
    assert pair["analysis"]["search_records"] == 1
    assert pair["evidence"]["strength_claim"] == "not_permitted"
    assert pair["evidence_status"] == {
        "raw_pair": "verified",
        "semantic_replay": "unknown",
        "evaluator": "unknown",
        "strength": "not_permitted",
    }
    assert pair["csa"]["sha256"] == record.sha256(csa)
    assert pair["analysis"]["sha256"] == record.sha256(sidecar)
    manifest = record.build_manifest(
        ROOT, ROOT / "scripts/fixtures", ROOT / "scripts/fixtures-does-not-exist",
        source_revision="fixture-revision", command_line="fixture command",
    )
    assert manifest["provenance"]["source_revision"] == "fixture-revision"
    assert manifest["provenance"]["binary"]["status"] == "not_supplied"
    assert manifest["provenance"]["weights"]["status"] == "not_supplied"
    assert manifest["run_contract"]["status"] == "not_supplied"
    assert manifest["preservation"] == {"status": "not_requested", "files": []}
    declared = record.build_manifest(
        ROOT, ROOT / "scripts/fixtures", ROOT / "scripts/fixtures-does-not-exist",
        run_contract={"status": "active", "evaluation": "material"},
    )
    assert declared["run_contract"]["status"] == "active"
    assert declared["run_contract"]["evaluation"] == "material"


def test_four_game_inventory_is_complete_when_local_data_is_available():
    csa_dir = ROOT / "data/floodgate"
    analysis_dir = ROOT / "data/floodgate-analysis"
    if not csa_dir.is_dir() or not analysis_dir.is_dir():
        return
    manifest = record.build_manifest(ROOT, csa_dir, analysis_dir)
    assert manifest["schema"] == "sekirei.floodgate-review-manifest.v1"
    assert manifest["summary"]["paired"] == 4
    assert manifest["summary"]["verified"] == 4
    assert all(pair["evidence"]["strength_claim"] == "not_permitted" for pair in manifest["pairs"])
    assert manifest["evidence_summary"]["raw_pair_verified"] == 4
    assert manifest["evidence_summary"]["evaluator_unknown"] == 4


def test_evidence_summary_counts_paired_fixture():
    manifest = record.build_manifest(
        ROOT, ROOT / "scripts/fixtures", ROOT / "scripts/fixtures",
    )
    assert manifest["evidence_summary"] == {
        "raw_pair_verified": 2,
        "raw_pair_invalid": 0,
        "semantic_replay_unknown": 2,
        "evaluator_verified": 0,
        "evaluator_unknown": 2,
        "strength_claim": "not_permitted",
    }


def test_missing_sidecar_is_explicit_and_non_destructive():
    # Use a deliberately absent directory so the test remains read-only on
    # managed runners where all temporary roots may be unavailable.
    csa_dir = ROOT / "scripts/fixtures"
    analysis_dir = ROOT / "scripts/fixtures-does-not-exist"
    manifest = record.build_manifest(ROOT, csa_dir, analysis_dir)
    assert manifest["summary"] == {"csa_files": 2, "paired": 0, "verified": 0, "invalid": 0, "missing": 2}
    assert manifest["missing_analysis"] == [
        "scripts/fixtures/analysis_replay_v1.csa",
        "scripts/fixtures/analysis_replay_v2.csa",
    ]


def test_cli_runtime_detection_is_recorded_without_shelling_out():
    with patch.object(record.platform, "uname", return_value=record.platform.uname()):
        assert record.detect_hardware()
    with patch.object(record.subprocess, "run", return_value=type(
        "Result", (), {"stdout": "rustc 1.99.0\n"}
    )()) as run:
        assert record.detect_toolchain() == "rustc 1.99.0"
        run.assert_called_once_with(
            ["rustc", "-Vv"], capture_output=True, text=True, check=True,
        )


if __name__ == "__main__":
    test_fixture_pair_has_integrity_record()
    test_four_game_inventory_is_complete_when_local_data_is_available()
    test_evidence_summary_counts_paired_fixture()
    test_missing_sidecar_is_explicit_and_non_destructive()
    test_cli_runtime_detection_is_recorded_without_shelling_out()
    print("PASS")
