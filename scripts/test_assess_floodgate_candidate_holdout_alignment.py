import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "alignment", ROOT / "scripts/assess_floodgate_candidate_holdout_alignment.py"
)
module = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(module)


def docs(release="v0.3.36"):
    return (
        {"schema": "sekirei.floodgate-review-manifest.v1", "run_contract": {"engine_version": "0.3.36"}},
        {"schema": "sekirei.release-manifest.v1", "release": release},
        {"schema": "sekirei.floodgate-holdout-regression.v1", "status": "clean_for_this_diagnostic"},
    )


def test_aligned():
    assert module.assess(*docs())["status"] == "aligned"


def test_rejects_historical_release():
    result = module.assess(*docs("v0.3.24"))
    assert result["status"] == "not_aligned"
    assert "holdout_release_version_mismatch" in result["reasons"]


if __name__ == "__main__":
    test_aligned()
    test_rejects_historical_release()
    print("PASS")
