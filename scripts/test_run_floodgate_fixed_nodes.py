import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("runner", ROOT / "scripts/run_floodgate_fixed_nodes.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def test_parse_output_keeps_last_completed_info_and_bestmove():
    parsed = MODULE.parse_output(
        "info depth 4 score 12 nodes 100 time 1\n"
        "info depth 5 score -8 nodes 200 time 2\n"
        "bestmove 7g7f\n"
    )
    assert parsed == {"bestmove_usi": "7g7f", "last_info": {"depth": 5, "score_cp": -8, "nodes": 200}}


def test_parse_output_does_not_invent_bestmove():
    assert MODULE.parse_output("info depth 1 score 0 nodes 1\n")["bestmove_usi"] is None


def test_parse_output_accepts_standard_usi_cp_score():
    parsed = MODULE.parse_output("info depth 3 score cp -42 nodes 999 time 1\nbestmove 9d9e\n")
    assert parsed["last_info"] == {"depth": 3, "score_cp": -42, "nodes": 999}


if __name__ == "__main__":
    test_parse_output_keeps_last_completed_info_and_bestmove()
    test_parse_output_does_not_invent_bestmove()
    test_parse_output_accepts_standard_usi_cp_score()
    print("PASS")
