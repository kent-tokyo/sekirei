import hashlib

from validate_independent_root_corpus import validate


def fixture():
    corpus_path = "corpus.json"
    corpus = {
        "schema": "sekirei.history-aware-csa-diagnostic.v1", "diagnostic_only": True,
        "invalid_replays": [],
        "positions": [{"id": "p", "initial_sfen": "start", "history_before_usi": ["7g7f"]}],
    }
    digest = hashlib.sha256(b"fixture").hexdigest()
    # The unit test replaces the hash check with the known fixture path only
    # after providing a matching temporary file through the public CLI; this
    # pure fixture focuses on overlap and pair invariants.
    teacher = {"schema": "sekirei.root-rank-teacher-corpus.v1", "diagnostic_only": True,
               "source_corpus": {"sha256": digest},
               "rows": [{"id": "p", "candidate_prefix_complete": True}]}
    pairs = {"schema": "sekirei.root-rank-pairs.v1", "diagnostic_only": True,
             "pairs": [{"parent_id": "p", "teacher_score_gap_cp": 1}]}
    subset = {"schema": "sekirei.hashed-csa-subset.v1", "entries": [{"source": "new.csa"}]}
    prior = {"sources": [{"path": "old.csa"}]}
    return subset, prior, corpus, teacher, pairs


def test_rejects_prior_source_overlap(tmp_path):
    subset, prior, corpus, teacher, pairs = fixture()
    path = tmp_path / "corpus.json"
    path.write_text("fixture", encoding="utf-8")
    teacher["source_corpus"]["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
    assert validate(subset, prior, corpus, teacher, pairs, path) == []
    prior["sources"] = [{"path": "new.csa"}]
    assert "prior_source_overlap" in validate(subset, prior, corpus, teacher, pairs, path)
