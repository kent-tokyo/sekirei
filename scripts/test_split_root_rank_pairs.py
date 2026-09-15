from split_root_rank_pairs import split


def pair(parent_id, parent_sfen, *, source=None, initial="start b - 1", history=None):
    return {
        "parent_id": parent_id,
        "parent_sfen": parent_sfen,
        "initial_sfen": initial,
        "history_before_usi": [] if history is None else history,
        "source": source,
        "category": "opening_control",
        "higher_move_usi": "7g7f",
        "lower_move_usi": "2g2f",
        "teacher_score_gap_cp": 1,
    }


document = {
    "schema": "sekirei.root-rank-pairs.v1",
    "diagnostic_only": True,
    "strength_claim": "not_permitted",
    "source_contract": {},
    "source_teacher": {},
    "pairs": [
        pair("game-a-ply000", "A b - 1", source={"replay_sha256": "game-a"}),
        pair("game-a-ply010", "B w - 11", source={"replay_sha256": "game-a"}, history=["7g7f"]),
        pair("game-b-ply000", "A b - 1", source={"replay_sha256": "game-b"}, history=["2g2f"]),
        pair("game-c-ply000", "C b - 1", source={"replay_sha256": "game-c"}, history=["3g3f"]),
        pair("legacy-ply000", "D b - 1", history=["4g4f"]),
        pair("legacy-ply003", "E w - 4", history=["9g9f"]),
    ],
}
train, valid, holdout, manifest = split(document, 300, 1, 300)
train_parents = {row["parent_id"] for row in train["pairs"]}
valid_parents = {row["parent_id"] for row in valid["pairs"]}
holdout_parents = {row["parent_id"] for row in holdout["pairs"]}
assert train_parents and valid_parents
assert train_parents.isdisjoint(valid_parents)
assert train_parents.isdisjoint(holdout_parents)
assert valid_parents.isdisjoint(holdout_parents)
assert {"game-a-ply000", "game-a-ply010"} <= train_parents or {"game-a-ply000", "game-a-ply010"} <= valid_parents
assert {"game-a-ply000", "game-b-ply000"} <= train_parents or {"game-a-ply000", "game-b-ply000"} <= valid_parents
assert {"legacy-ply000", "legacy-ply003"} <= train_parents or {"legacy-ply000", "legacy-ply003"} <= valid_parents
assert manifest["component_leakage"] == 0
assert manifest["split_unit"] == "connected_source_game_prefix_or_position"
assert manifest["holdout_pairs"] == len(holdout["pairs"])
assert manifest["train_sha256"]
print("PASS")
