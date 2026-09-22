import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("gate_execution", ROOT / "scripts/record_strength_gate_execution.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def checkpoint_hash(data: bytes) -> str:
    value = 14695981039346656037
    for byte in data:
        value = ((value ^ byte) * 1099511628211) & ((1 << 64) - 1)
    return f"{value:016x}"


def write_weight(path: Path, data: bytes) -> None:
    path.write_bytes(data)
    path.with_suffix(".meta.json").write_text(json.dumps({
        "format": "sekirei-nnue-output-v1",
        "nnue_output": "absolute",
        "baseline": None,
        "checkpoint_hash": checkpoint_hash(data),
    }), encoding="utf-8")


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    candidate = root / "candidate.bin"
    baseline = root / "baseline.bin"
    write_weight(candidate, b"candidate")
    write_weight(baseline, b"baseline")
    plan = root / "plan.json"
    plan.write_text(json.dumps({
        "schema": "sekirei.strength-gate-plan.v1", "status": "planned", "strength_claim": False,
        "candidate": {"path": str(candidate), "sha256": "candidate"},
        "baseline": {"path": str(baseline), "sha256": "baseline"},
        "evaluation": {
            "candidate": {"mode": "absolute", "weights": {"path": str(candidate)},
                          "checkpoint_hash": checkpoint_hash(b"candidate")},
            "baseline": {"mode": "absolute", "weights": {"path": str(baseline)},
                         "checkpoint_hash": checkpoint_hash(b"baseline")},
        },
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
                                             "option1": [f"EvalFile={candidate}", "NnueOutput=absolute", "Threads=1", "SpecTopN=0", "UseBook=false", "SearchMode=Speculative"],
                                             "option2": [f"EvalFile={baseline}", "NnueOutput=absolute", "Threads=1", "SpecTopN=0", "UseBook=false", "SearchMode=Speculative"]},
                                 "shards": [{"status": "running"}], "confirmed_prefix": 0,
                                 "decisive_verdict": None}), encoding="utf-8")
    document = MODULE.build(plan, binary, preflight, state)
    assert document["status"] == "running"
    assert document["strength_claim"] is False
    assert document["state_at_recording"]["confirmed_prefix"] == 0
    assert document["binary"]["sha256"] == MODULE.sha256(binary)
    state.write_text(json.dumps({"cfg": {"threads": 2, "parallel": 1, "byoyomi": 1000,
                                             "corpus": "openings.sfen",
                                             "option1": [f"EvalFile={candidate}"],
                                             "option2": [f"EvalFile={baseline}"]},
                                 "shards": [{"status": "running"}]}), encoding="utf-8")
    try:
        MODULE.build(plan, binary, preflight, state)
    except ValueError as error:
        assert "thread/parallel/time" in str(error)
    else:
        raise AssertionError("mismatched state unexpectedly recorded")
print("PASS")
