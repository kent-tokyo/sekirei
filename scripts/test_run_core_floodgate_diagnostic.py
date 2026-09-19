import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("runner", ROOT / "scripts/run_core_floodgate_diagnostic.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)
ASSERTIONS = unittest.TestCase()


def test_parse_core_result():
    assert MODULE.parse_result("bestmove=3c4e\tdepth=4\tscore_cp=200\tnodes=20\telapsed_ms=5\tcompleted_bound=exact\tcompleted_iteration_valid=true") == {
        "bestmove": "3c4e", "depth": 4, "score_cp": 200, "nodes": 20, "elapsed_ms": 5,
        "completed_bound": "exact", "completed_iteration_valid": "true", "root_candidates": [],
    }


def test_build_command_propagates_nmp_ablation_only_when_requested():
    base = MODULE.build_command(Path("engine"), "startpos", 100, None)
    disabled = MODULE.build_command(Path("engine"), "startpos", 100, None, disable_nmp=True)
    assert "--disable-nmp" not in base
    assert disabled[-1] == "--disable-nmp"


def test_build_command_propagates_lmr_ablation_only_when_requested():
    base = MODULE.build_command(Path("engine"), "startpos", 100, None)
    disabled = MODULE.build_command(Path("engine"), "startpos", 100, None, disable_lmr=True)
    assert "--disable-lmr" not in base
    assert disabled[-1] == "--disable-lmr"


def test_build_command_propagates_fixed_depth_mode():
    command = MODULE.build_command(Path("engine"), "startpos", 100, None, max_depth=2)
    assert command[-2:] == ["--max-depth", "2"]


def test_build_command_binds_residual_output_mode_to_loaded_weights():
    command = MODULE.build_command(
        Path("engine"), "startpos", 100, Path("candidate.bin"),
        nnue_output="residual-material",
    )
    assert command[command.index("--nnue-output") + 1] == "residual-material"


def test_build_command_propagates_root_candidate_opt_in():
    command = MODULE.build_command(Path("engine"), "startpos", 100, None, root_candidates=3)
    assert "--root-candidates" in command
    assert command[command.index("--root-candidates") + 1] == "3"


def test_build_command_replays_history_before_search_and_checks_result_position():
    command = MODULE.build_command(
        Path("engine"), "initial-sfen", 100, None,
        history_moves_usi=["7g7f", "3c3d"], expected_sfen="expected-sfen",
    )
    assert command[command.index("--moves") + 1] == "7g7f 3c3d"
    assert command[command.index("--expected-sfen") + 1] == "expected-sfen"


def test_build_command_preserves_an_explicit_empty_history():
    command = MODULE.build_command(
        Path("engine"), "initial-sfen", 100, None,
        history_moves_usi=[], expected_sfen="initial-sfen",
    )
    assert command[command.index("--moves") + 1] == ""


def test_parse_core_result_keeps_bound_abort_and_pv_fields():
    result = MODULE.parse_result(
        "bestmove=3c4e\tdepth=4\tscore_cp=200\tnodes=20\telapsed_ms=5\tcompleted_bound=exact\tcompleted_iteration_valid=true"
        "\tbound=exact\taborted=false\tpv_usi=3c4e,7g7f"
    )
    assert result["bound"] == "exact"
    assert result["aborted"] == "false"
    assert result["pv_usi"] == "3c4e,7g7f"


def test_parse_core_result_normalizes_pv_integrity_booleans():
    result = MODULE.parse_result(
        "bestmove=3c4e\tdepth=4\tscore_cp=200\tnodes=20\telapsed_ms=5\tcompleted_bound=exact\tcompleted_iteration_valid=true"
        "\tpv_legal=true\tpv_replay_preserves_input=false"
    )
    assert result["pv_legal"] is True
    assert result["pv_replay_preserves_input"] is False


def test_parse_core_result_structures_opt_in_root_candidates():
    result = MODULE.parse_result(
        "bestmove=3c4e\tdepth=2\tscore_cp=20\tnodes=64\telapsed_ms=5\tcompleted_bound=exact\tcompleted_iteration_valid=true"
        "\troot_candidates=7g7f:20:2:exact:none,2g2f:-10:1:unknown:budget\troot_legal_move_count=12"
    )
    assert result["root_candidates"] == [
        {"move": "7g7f", "score_cp": 20, "depth": 2, "bound": "exact", "abort_reason": "none"},
        {"move": "2g2f", "score_cp": -10, "depth": 1, "bound": "unknown", "abort_reason": "budget"},
    ]
    assert result["root_legal_move_count"] == 12


def test_parse_core_result_rejects_malformed_root_candidate():
    with ASSERTIONS.assertRaisesRegex(ValueError, "invalid root_candidates"):
        MODULE.parse_result("bestmove=3c4e\tdepth=1\tcompleted_bound=exact\tcompleted_iteration_valid=true\troot_candidates=broken")


def test_parse_core_result_rejects_duplicate_root_candidate():
    with ASSERTIONS.assertRaisesRegex(ValueError, "duplicate root candidate"):
        MODULE.parse_result(
            "bestmove=3c4e\tdepth=1\tcompleted_bound=exact\tcompleted_iteration_valid=true\troot_candidates=7g7f:20:1:exact:none,7g7f:10:1:exact:none"
        )


def test_csa_move_to_usi_handles_normal_promotion_and_drop():
    sfen = "9/9/9/9/9/9/9/1B7/9 b R 1"
    assert MODULE.csa_move_to_usi("+7776FU", sfen) == "7g7f"
    assert MODULE.csa_move_to_usi("+0055HI", sfen) == "R*5e"
    assert MODULE.csa_move_to_usi("+8822UM", sfen) == "8h2b+"


def test_csa_move_to_usi_does_not_repromote_an_already_promoted_piece():
    sfen = "9/9/9/9/9/9/9/1+B7/9 b - 1"
    assert MODULE.csa_move_to_usi("+8822UM", sfen) == "8h2b"


def test_csa_move_to_usi_rejects_malformed_tokens():
    try:
        MODULE.csa_move_to_usi("+7776XX", "9/9/9/9/9/9/9/9/9 b - 1")
    except ValueError:
        pass
    else:
        raise AssertionError("malformed CSA token was accepted")


def test_history_fingerprint_is_order_sensitive_and_stable():
    first = MODULE.history_fingerprint(["+7776FU", "-3334FU"])
    second = MODULE.history_fingerprint(["-3334FU", "+7776FU"])
    assert first != second
    assert first == MODULE.history_fingerprint(["+7776FU", "-3334FU"])


def test_summarize_tt_separates_cold_and_warm_values():
    summary = MODULE.summarize_tt({
        "completion": "search_completed", "warmup_bestmove": "7g7f",
        "bestmove": "2g2f", "warmup_depth": 2, "depth": 3,
        "warmup_score_cp": 10, "score_cp": 30,
        "warmup_nodes": 100, "nodes": 100,
    })
    assert summary["status"] == "diagnostic_only"
    assert summary["bestmove_changed"] is True
    assert summary["score_delta_warm_minus_cold_cp"] == 20
    assert summary["same_node_budget"] is True


def test_run_corpus_selects_only_explicit_unique_entry_indices():
    corpus = {
        "entries": [
            {"source": {"game_id": f"g{index}", "ply": index}, "position": {"sfen": "s", "history_before": []}}
            for index in range(3)
        ]
    }
    fake_result = {
        "completion": "search_completed", "bestmove": "7g7f", "score_cp": 0,
        "depth": 1, "nodes": 1, "elapsed_ms": 1,
        "completed_iteration_valid": "true", "completed_bound": "exact",
    }
    with patch.object(MODULE, "run_position", return_value=fake_result):
        result = MODULE.run_corpus(Path("engine"), corpus, 1, 1, None, None, entry_indices=[2, 0])
        assert [item["source"]["game_id"] for item in result["results"]] == ["g2", "g0"]
        with ASSERTIONS.assertRaisesRegex(ValueError, "unique"):
            MODULE.run_corpus(Path("engine"), corpus, 1, 1, None, None, entry_indices=[0, 0])
        with ASSERTIONS.assertRaisesRegex(ValueError, "outside"):
            MODULE.run_corpus(Path("engine"), corpus, 1, 1, None, None, entry_indices=[3])


def test_history_aware_corpus_normalizes_replayable_usi_context():
    corpus = {
        "schema": "sekirei.history-aware-csa-diagnostic.v1",
        "positions": [{
            "id": "g-ply1", "category": "after_drop", "sfen": "final",
            "initial_sfen": "initial", "history_before_usi": ["7g7f"],
            "source": {"ply": 1},
        }],
    }
    entry = MODULE.diagnostic_entries(corpus)[0]
    assert entry["source"] == {"game_id": "g-ply1", "category": "after_drop", "ply": 1}
    assert entry["position"]["initial_sfen"] == "initial"
    assert entry["position"]["history_before_usi"] == ["7g7f"]


def test_history_aware_source_without_raw_csa_does_not_invoke_legacy_replay():
    result = MODULE.optional_history_replay(
        Path("history-replay"),
        {"replay": "replay.json", "replay_sha256": "a" * 64, "ply": 1},
        "final",
        1,
    )
    assert result == {
        "status": "not_run",
        "reason": "raw CSA source unavailable; core USI history is authoritative",
    }


def test_run_corpus_marks_core_history_replay_only_when_v2_fields_are_complete():
    corpus = {"entries": [{
        "source": {"game_id": "g", "ply": 1},
        "position": {
            "sfen": "final", "initial_sfen": "initial",
            "history_before": ["+7776FU"], "history_before_usi": ["7g7f"],
        },
    }]}
    calls = []

    def fake_run(*args, **kwargs):
        calls.append((args, kwargs))
        return {
            "completion": "search_completed", "bestmove": "7g7f", "score_cp": 0,
            "depth": 1, "nodes": 1, "elapsed_ms": 1,
            "completed_iteration_valid": "true", "completed_bound": "exact",
            "history_replayed": True,
        }

    with patch.object(MODULE, "run_position", fake_run):
        result = MODULE.run_corpus(Path("engine"), corpus, 1, 1, None, None)
    assert result["results"][0]["history"]["replayed_into_core"] is True
    assert calls[0][0][1] == "initial"
    assert calls[0][1]["history_moves_usi"] == ["7g7f"]
    assert calls[0][1]["expected_sfen"] == "final"


def test_run_corpus_accepts_direct_usi_observed_move():
    corpus = {"entries": [{
        "source": {"game_id": "gate-loss", "ply": 0},
        "position": {"sfen": "final", "actual_move_usi": "7g7f", "history_before": []},
    }]}
    calls = []

    def fake_run(*args, **kwargs):
        calls.append((args, kwargs))
        return {
            "completion": "search_completed", "bestmove": "7g7f", "score_cp": 0,
            "depth": 1, "nodes": 1, "elapsed_ms": 1,
            "completed_iteration_valid": "true", "completed_bound": "exact",
        }

    with patch.object(MODULE, "run_position", fake_run):
        result = MODULE.run_corpus(Path("engine"), corpus, 1, 1, None, None)
    assert result["results"][0]["observed_move_usi"] == "7g7f"
    assert result["results"][0]["diagnostic"]["forcing_class"] == "unclassified"
    assert len(calls) == 2


def test_rejects_incomplete_core_result():
    try:
        MODULE.parse_result("bestmove=3c4e\tscore_cp=200")
    except ValueError:
        pass
    else:
        raise AssertionError("incomplete result was accepted")


if __name__ == "__main__":
    for name, test in sorted(globals().copy().items()):
        if name.startswith("test_") and callable(test):
            test()
    print("PASS")
