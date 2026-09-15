import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("gate_execution", ROOT / "scripts/record_strength_gate_execution.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    plan = root / "plan.json"
    plan.write_text(json.dumps({
        "schema": "sekirei.strength-gate-plan.v1", "status": "planned", "strength_claim": False,
        "candidate": {"path": "candidate.bin", "sha256": "candidate"},
        "baseline": {"path": "baseline.bin", "sha256": "baseline"},
        "openings": {"path": "openings.sfen", "sha256": "openings"},
        "protocol": {
            "games_per_position": 2, "positions": 200, "max_games": 400,
            "sprt": {"variant": "trinomial", "paired_by_id": True},
            "byoyomi_ms": 1000,
            "engine_options": {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative"},
        },
    }), encoding="utf-8")
    binary = root / "engine"
    binary.write_bytes(b"engine")
    preflight = root / "preflight.json"
    preflight.write_text("{}", encoding="utf-8")
    state = root / "state.json"
    state.write_text(json.dumps({"cfg": {"threads": 1, "parallel": 1, "byoyomi": 1000,
                                             "corpus": "openings.sfen",
                                             "option1": ["EvalFile=candidate.bin", "Threads=1", "SpecTopN=0", "UseBook=false", "SearchMode=Speculative"],
                                             "option2": ["EvalFile=baseline.bin", "Threads=1", "SpecTopN=0", "UseBook=false", "SearchMode=Speculative"]},
                                 "shards": [{"status": "running"}], "confirmed_prefix": 0,
                                 "decisive_verdict": None}), encoding="utf-8")
    document = MODULE.build(plan, binary, preflight, state)
    assert document["status"] == "running"
    assert document["strength_claim"] is False
    assert document["state_at_recording"]["confirmed_prefix"] == 0
    assert document["binary"]["sha256"] == MODULE.sha256(binary)
    state.write_text(json.dumps({"cfg": {"threads": 2, "parallel": 1, "byoyomi": 1000,
                                             "corpus": "openings.sfen",
                                             "option1": ["EvalFile=candidate.bin"],
                                             "option2": ["EvalFile=baseline.bin"]},
                                 "shards": [{"status": "running"}]}), encoding="utf-8")
    try:
        MODULE.build(plan, binary, preflight, state)
    except ValueError as error:
        assert "thread/parallel/time" in str(error)
    else:
        raise AssertionError("mismatched state unexpectedly recorded")
print("PASS")
