import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("attach", ROOT / "scripts/attach_semantic_replay.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_attach_requires_exact_pair_and_record_counts():
    manifest = {
        "schema": "sekirei.floodgate-review-manifest.v1",
        "pairs": [{
            "id": "g",
            "csa": {"moves": 3},
            "analysis": {"search_records": 2},
            "evidence": {"semantic_replay": "unknown"},
            "evidence_status": {"semantic_replay": "unknown"},
        }],
        "evidence_summary": {},
    }
    replay = {"results": [{"id": "g", "status": "verified", "csa_moves": 3, "search_records": 2}]}
    result = module.attach(manifest, replay)
    assert result["pairs"][0]["evidence_status"]["semantic_replay"] == "verified"
    assert result["evidence_summary"]["semantic_replay_verified"] == 1
    assert manifest["pairs"][0]["evidence_status"]["semantic_replay"] == "unknown"


def test_attach_rejects_count_mismatch():
    manifest = {
        "schema": "sekirei.floodgate-review-manifest.v1",
        "pairs": [{"id": "g", "csa": {"moves": 3}, "analysis": {"search_records": 2},
                   "evidence": {}, "evidence_status": {}}],
        "evidence_summary": {},
    }
    replay = {"results": [{"id": "g", "status": "verified", "csa_moves": 4, "search_records": 2}]}
    try:
        module.attach(manifest, replay)
    except ValueError as exc:
        assert "CSA move count mismatch" in str(exc)
    else:
        raise AssertionError("count mismatch was accepted")


if __name__ == "__main__":
    test_attach_requires_exact_pair_and_record_counts()
    test_attach_rejects_count_mismatch()
    print("PASS")
