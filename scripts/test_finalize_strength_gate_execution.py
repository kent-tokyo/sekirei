import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("finalize_gate", ROOT / "scripts/finalize_strength_gate_execution.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    combined = root / "combined.json"
    combined.write_text(json.dumps({"games": 2}), encoding="utf-8")
    combined_jsonl = root / "combined.jsonl"
    combined_jsonl.write_text(
        '{"id":"pos0_pair0","result":"candidate_win"}\n'
        '{"id":"pos0_pair0","result":"baseline_win"}\n',
        encoding="utf-8",
    )
    execution = root / "execution.json"
    execution.write_text(json.dumps({
        "schema": "sekirei.strength-gate-execution.v1", "run_directory": str(root),
        "candidate": {"sha256": "candidate"}, "baseline": {"sha256": "baseline"},
        "evaluation": {"candidate": {"mode": "residual-material"}, "baseline": {"mode": "residual-material"}},
        "protocol": {
            "games_per_position": 2, "positions": 200, "max_games": 400,
            "sprt": {"variant": "trinomial", "paired_by_id": True},
        },
    }), encoding="utf-8")
    state = root / "state.json"
    state.write_text(json.dumps({"confirmed_prefix": 1, "decisive_verdict": "PASS", "shards": [{"status": "done"}]}), encoding="utf-8")
    document = MODULE.build(execution, state)
    assert document["verdict"] == "PASS"
    assert document["games"] == 2
    state.write_text(json.dumps({"confirmed_prefix": 1, "decisive_verdict": None, "shards": [{"status": "running"}]}), encoding="utf-8")
    try:
        MODULE.build(execution, state)
    except ValueError as error:
        assert "still running" in str(error)
    else:
        raise AssertionError("running gate was finalized")
print("PASS")
