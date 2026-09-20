import importlib.util
import json
from pathlib import Path
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("gate_openings", ROOT / "scripts/build_gate_openings_from_csa.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def position(ply, sfen="sfen", history=None):
    return {"ply": ply, "pre_move_sfen": sfen, "history_before_usi": history or ["7g7f"]}


def test_select_position_is_deterministic_and_excludes_opening():
    document = {"schema": "sekirei.csa-replay.v2", "positions": [
        position(0, "opening"), position(12, "first"), position(81, "too-late"), position(20, "second"),
    ]}
    selected = MODULE.select_position(document, "a" * 64)
    assert selected is not None
    assert selected["pre_move_sfen"] in {"first", "second"}
    assert MODULE.select_position(document, "a" * 64) == selected


def test_select_position_rejects_malformed_history_or_wrong_schema():
    assert MODULE.select_position({"schema": "wrong", "positions": []}, "b" * 64) is None
    malformed = {"schema": "sekirei.csa-replay.v2", "positions": [
        {"ply": 12, "pre_move_sfen": "x", "history_before_usi": [""]},
    ]}
    assert MODULE.select_position(malformed, "b" * 64) is None


def test_excluded_source_paths_supports_prior_manifest_shapes():
    with tempfile.TemporaryDirectory() as directory:
        manifest = Path(directory) / "prior.json"
        manifest.write_text(json.dumps({
            "sources": [{"path": "data/csa/2025/old-a.csa"}],
            "entries": [{"source": "data/csa/2025/old-b.csa"}],
            "selected": [{"path": "data/csa/2025/old-c.csa"}],
        }), encoding="utf-8")
        excluded = MODULE.excluded_source_paths([manifest])
        for name in ("old-a.csa", "old-b.csa", "old-c.csa"):
            assert f"data/csa/2025/{name}" in excluded
