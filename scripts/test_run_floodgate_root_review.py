#!/usr/bin/env python3
import importlib.util
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("root_review", ROOT / "scripts/run_floodgate_root_review.py")
root_review = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(root_review)


def test_runner_emits_v2_budget_contract():
    with tempfile.TemporaryDirectory() as directory:
        temp = Path(directory)
        binary = temp / "engine"
        history_binary = temp / "history"
        csa = temp / "game.csa"
        analysis = temp / "game.analysis.jsonl"
        for path in (binary, history_binary, csa, analysis):
            path.write_bytes(path.name.encode("utf-8"))

        root_review.select_positions = lambda *_args: [{
            "game_id": "fixture-game",
            "result": "win",
            "csa": csa,
            "analysis": analysis,
            "ply": 2,
            "sfen": "4k4/9/9/9/9/9/9/9/4K4 b - 1",
            "played_move_csa": "+7776FU",
            "source_score_delta_cp": 20,
        }]
        root_review.csa_move_to_usi = lambda *_args: "7g7f"
        root_review.run_history_replay = lambda *_args: {"status": "verified"}

        def fake_search(_binary, _sfen, root_move, warmup, nodes, depth, _disable_nmp):
            result = {
                "bestmove": root_move or "7g7f",
                "depth": depth,
                "score_cp": 0,
                "nodes": nodes,
                "completion": "search_completed",
            }
            if root_move:
                result["root_move_usi"] = root_move
            return result

        root_review.search = fake_search
        document = root_review.review(binary, history_binary, temp, temp, 20_000, 2, 1)

    assert document["schema"] == "sekirei.floodgate-root-review.v2"
    comparison = document["rows"][0]["comparison"]
    assert comparison["requested_node_budget"] == 20_000
    assert comparison["same_requested_budget"] is True
    assert document["rows"][0]["actual_root"]["root_move_usi"] == "7g7f"


if __name__ == "__main__":
    test_runner_emits_v2_budget_contract()
    print("PASS")
