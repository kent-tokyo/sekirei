import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("plan", ROOT / "scripts/prepare_floodgate_candidate_holdout_plan.py")
module = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(module)


def test_prepares_non_executing_plan():
    result = module.prepare(
        {"schema": "sekirei.floodgate-review-manifest.v1", "run_contract": {"engine_version": "0.3.36"},
         "provenance": {"binary": {"sha256": "bin"}, "source_revision": "worktree"}},
        {"schema": "sekirei.floodgate-diagnostic-corpus.v1"},
        {"schema": "sekirei.floodgate-diagnostic-corpus-split.v1", "source_corpus_sha256": "corpus",
         "tuning_entry_indices": [0], "holdout_entry_indices": [1, 2]},
        {"candidate": "candidate.json", "corpus": "corpus.json", "split": "split.json", "corpus_sha256": "file"},
    )
    assert result["status"] == "planned"
    assert result["execution_performed"] is False
    assert result["protocol"]["preflight_required"] is True
    assert result["split"]["holdout_entries"] == 2


if __name__ == "__main__":
    test_prepares_non_executing_plan()
    print("PASS")
