import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_csa_run_manifest import validate


def base(evaluation="material", active=False):
    return {
        "schema": "sekirei.csa-run-manifest.v1", "status": "active", "engine": "sekirei",
        "engine_version": "0.3.36", "source_revision": "abc", "evaluation": evaluation,
        "search_backend": "alpha_beta", "hash_mb": 256, "max_depth": 50,
        "resign_score_cp": -2000, "ponder": "disabled", "game_id": "g",
        "server": "localhost", "port": 4081,
        "binary": {"path": "/bin/engine", "bytes": 1, "sha256": "deferred_to_finalize_csa_run_manifest"},
        "weights": {"path": None, "active": active, "sha256": "deferred_to_finalize_csa_run_manifest"},
    }


assert validate(base()) == []
explicit_scale = base("nnue", True)
explicit_scale["weights"]["path"] = "/tmp/synthetic-halfkp.bin"
explicit_scale["fv_scale"] = 24
assert validate(explicit_scale) == []
for value in (0, 129, True, False, None, "24", 24.0):
    invalid_scale = base()
    invalid_scale["fv_scale"] = value
    assert "fv_scale" in validate(invalid_scale)
bounded = base()
bounded.update({"keep_alive": True, "max_games": 5, "completed_attempts": 5,
                "terminal_stop_reason": "max_games_reached"})
assert validate(bounded) == []
failed = base()
failed.update({
    "status": "failed",
    "completed_attempts": 2,
    "max_games": 10,
    "terminal_stop_reason": "child_exited_before_max_games",
    "child_exit_code": 3,
    "incomplete_game": {
        "game_id": "g-live",
        "classification": "incomplete",
        "counted": False,
        "last_client_state": "in_game",
    },
})
assert validate(failed) == []
failed["incomplete_game"]["counted"] = True
assert "incomplete_game" in validate(failed)
invalid_limit = base()
invalid_limit["max_games"] = 0
assert "max_games" in validate(invalid_limit)
assert "weights.nnue_active" in validate(base("nnue"))
assert "weights.material_active" in validate(base("material", True))
finalized = base()
finalized["status"] = "finalized"
finalized["binary"]["sha256"] = "a" * 64
finalized["weights"]["sha256"] = "b" * 64
assert validate(finalized, finalized=True) == []
material_without_weights = base()
material_without_weights["status"] = "finalized"
material_without_weights["binary"]["sha256"] = "a" * 64
material_without_weights["weights"] = {"status": "unknown", "active": False}
assert validate(material_without_weights, finalized=True) == []
nnue_without_weight_hash = base("nnue", True)
nnue_without_weight_hash["status"] = "finalized"
nnue_without_weight_hash["binary"]["sha256"] = "a" * 64
nnue_without_weight_hash["weights"]["sha256"] = "deferred_to_finalize_csa_run_manifest"
assert "weights.sha256" in validate(nnue_without_weight_hash, finalized=True)
print("PASS")
