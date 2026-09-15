import json

from build_root_rank_teacher_corpus import select_positions


corpus = {
    "diagnostic_only": True,
    "positions": [
        {"id": "opening-a", "category": "opening", "initial_sfen": "start", "history_before_usi": [], "sfen": "start"},
        {"id": "opening-duplicate", "category": "opening", "initial_sfen": "start", "history_before_usi": [], "sfen": "start"},
        {"id": "drop-a", "category": "drop", "initial_sfen": "start", "history_before_usi": ["7g7f"], "sfen": "after"},
        {"id": "drop-b", "category": "drop", "initial_sfen": "start", "history_before_usi": ["2g2f"], "sfen": "after2"},
    ],
}
selected = select_positions(corpus, 1)
assert [entry["id"] for entry in selected] == ["opening-a", "drop-a"]
try:
    select_positions({"diagnostic_only": False, "positions": []}, 1)
except ValueError as error:
    assert "diagnostic corpus" in str(error)
else:
    raise AssertionError("non-diagnostic corpus unexpectedly accepted")
print(json.dumps({"status": "PASS", "selected": [entry["id"] for entry in selected]}))
