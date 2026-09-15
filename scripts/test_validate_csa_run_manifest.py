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
