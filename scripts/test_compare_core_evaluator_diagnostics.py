import json
import tempfile
from pathlib import Path

from compare_core_evaluator_diagnostics import compare


def doc(scores, eval_mode):
    return {"schema": "sekirei.floodgate-core-diagnostic.v3", "diagnostic_only": True,
            "eval_mode": eval_mode,
            "results": [{"source": {"game_id": "g", "ply": i},
                          "unrestricted": {"completion": "search_completed", "score_cp": score,
                                            "bestmove": move, "elapsed_ms": 2,
                                            "completed_iteration_valid": "true",
                                            "completed_bound": "exact"}}
                         for i, (score, move) in enumerate(scores)]}


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root / "m.json").write_text(json.dumps(doc([(100, "7g7f"), (-50, "2g2f")], "material")), encoding="utf-8")
    (root / "n.json").write_text(json.dumps(doc([(80, "7g7f"), (50, "2g2f")], "king-safety")), encoding="utf-8")
    result = compare(root / "m.json", root / "n.json")
    assert result["summary"]["comparable"] == 2
    assert result["summary"]["bestmove_changes"] == 0
    assert result["summary"]["sign_mismatches"] == 1
    assert result["summary"]["score_sign_partition"] == "positive_vs_non_positive"
    assert result["summary"]["score_pearson"] == 1.0
    assert result["inputs"]["candidate_eval_mode"] == "king-safety"

    (root / "teacher.json").write_text(
        json.dumps(doc([(100, "7g7f"), (-50, "2g2f")], "nnue-residual-material")),
        encoding="utf-8",
    )
    generic = compare(root / "teacher.json", root / "n.json", allow_nonmaterial_baseline=True)
    assert generic["inputs"]["baseline_eval_mode"] == "nnue-residual-material"
    assert generic["summary"]["comparable"] == 2
print("PASS")
