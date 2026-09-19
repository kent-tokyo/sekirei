from build_root_rank_pairs import pairs


document = {
    "schema": "sekirei.root-rank-teacher-corpus.v1",
    "diagnostic_only": True,
    "contract": {"normal_score_abs_max_cp": 10_000},
    "rows": [
        {"id": "a", "category": "opening", "candidate_prefix_complete": True, "complete_legal_root_set": True,
         "source": {"replay_sha256": "source-a"},
         "teacher_root": {"root_candidates": [
             {"move": "7g7f", "score_cp": 10},
             {"move": "2g2f", "score_cp": 3},
             {"move": "3g3f", "score_cp": 3},
         ]}},
        {"id": "incomplete", "candidate_prefix_complete": False,
         "teacher_root": {"root_candidates": [{"move": "4g4f", "score_cp": 1}]}},
    ],
}
labels = pairs(document)
assert len(labels) == 2
assert {(label["higher_move_usi"], label["lower_move_usi"], label["teacher_score_gap_cp"]) for label in labels} == {
    ("7g7f", "2g2f", 7), ("7g7f", "3g3f", 7),
}
assert labels[0]["source"] == {"replay_sha256": "source-a"}

adjacent_labels = pairs(document, "adjacent")
assert len(adjacent_labels) == 1
assert {
    (label["higher_move_usi"], label["lower_move_usi"], label["teacher_score_gap_cp"])
    for label in adjacent_labels
} == {("7g7f", "2g2f", 7)}
try:
    pairs(document, top_k=1)
except ValueError as error:
    assert "top_k" in str(error)
else:
    raise AssertionError("top_k=1 unexpectedly accepted")
print("PASS")
