import json
import tempfile
from pathlib import Path

from compare_core_search_config_diagnostics import compare


def document(scores, schema="sekirei.floodgate-core-diagnostic.v2"):
    return {
        "schema": schema,
        "diagnostic_only": True,
        "nodes": 20_000,
        "execution": {
            "binary": {"sha256": "binary"}, "weights": {"sha256": "weights"},
            "corpus_sha256": "corpus",
            "options": {"threads": 1, "spec_top_n": 0, "use_book": False,
                        "tt_mode": "cold_process", "nnue_output": "residual-material"},
        },
        "results": [
            {
                "source": {"game_id": "g", "ply": index},
                "unrestricted": {
                    "completion": "search_completed",
                    "completed_iteration_valid": "true",
                    "completed_bound": "exact",
                    "score_cp": score,
                    "depth": 4,
                    "bestmove": move,
                    "elapsed_ms": elapsed_ms,
                },
            }
            for index, (score, move, elapsed_ms) in enumerate(scores)
        ],
    }


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root / "left.json").write_text(json.dumps(document([(10, "7g7f", 2), (-5, "2g2f", 4)])), encoding="utf-8")
    (root / "right.json").write_text(json.dumps(document([(8, "7g7f", 3), (5, "3g3f", 2)])), encoding="utf-8")
    result = compare(root / "left.json", root / "right.json", "cold", "warm")
    assert result["summary"]["comparable"] == 2
    assert result["summary"]["bestmove_changes"] == 1
    assert result["summary"]["sign_mismatches"] == 1
    assert result["summary"]["elapsed_ratio_right_over_left_mean"] == 1.0
    assert result["inputs"]["left_label"] == "cold"

    # v3 adds evaluator metadata but keeps the completed-result contract.
    (root / "v3.json").write_text(
        json.dumps(document([(8, "7g7f", 3), (5, "3g3f", 2)], "sekirei.floodgate-core-diagnostic.v3")),
        encoding="utf-8",
    )
    v3_result = compare(root / "left.json", root / "v3.json", "control", "freeze")
    assert v3_result["summary"]["comparable"] == 2
print("PASS")
