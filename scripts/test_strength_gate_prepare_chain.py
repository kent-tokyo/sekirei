"""Exercise frozen-gate preparation without building or launching a game.

The test uses a throwaway plan and fake engine files.  It proves that the
durable state is initialized before the execution manifest is recorded, and
that the record binds the state configuration to the immutable plan.
"""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


def checkpoint_hash(path: Path) -> str:
    value = 14695981039346656037
    for byte in path.read_bytes():
        value = ((value ^ byte) * 1099511628211) & ((1 << 64) - 1)
    return f"{value:016x}"


def evaluation(path: Path) -> dict[str, object]:
    sidecar = path.with_suffix(".meta.json")
    sidecar.write_text(json.dumps({
        "format": "sekirei-nnue-output-v1",
        "nnue_output": "residual-material",
        "baseline": "material-v1",
        "checkpoint_hash": checkpoint_hash(path),
    }), encoding="utf-8")
    return {
        "weights": {"path": str(path), "sha256": "fixture"},
        "metadata": {"path": str(sidecar), "sha256": "fixture"},
        "mode": "residual-material",
        "baseline": "material-v1",
        "checkpoint_hash": checkpoint_hash(path),
    }


ROOT = Path(__file__).resolve().parents[1]


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    candidate, baseline, engine = root / "candidate.bin", root / "baseline.bin", root / "engine"
    for path in (candidate, baseline, engine):
        path.write_bytes(b"fixture")
    openings = root / "openings.sfen"
    openings.write_text("startpos\n", encoding="utf-8")
    plan = root / "plan.json"
    candidate_evaluation, baseline_evaluation = evaluation(candidate), evaluation(baseline)
    plan.write_text(json.dumps({
        "schema": "sekirei.strength-gate-plan.v1", "status": "planned", "strength_claim": False,
        "candidate": {"path": str(candidate), "sha256": "candidate"},
        "baseline": {"path": str(baseline), "sha256": "baseline"},
        "evaluation": {"candidate": candidate_evaluation, "baseline": baseline_evaluation},
        "openings": {"path": str(openings), "sha256": "openings"},
        "protocol": {
            "games_per_position": 2, "positions": 200, "max_games": 400, "byoyomi_ms": 1000,
            "engine_options": {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative"},
            "sprt": {"variant": "trinomial", "paired_by_id": True},
        },
    }), encoding="utf-8")
    outdir = root / "run"
    command = [
        sys.executable, str(ROOT / "scripts/gate_orchestrator.py"), "run",
        "--outdir", str(outdir), "--threads", "1", "--parallel", "1", "--byoyomi", "1000",
        "--shard-positions", "1", "--max-positions", "1", "--engine-bin", str(engine),
        "--corpus", str(openings), "--option1", f"EvalFile={candidate}", "--option1", "NnueOutput=residual-material",
        "--option2", f"EvalFile={baseline}", "--option2", "NnueOutput=residual-material",
        "--option1", "Threads=1", "--option1", "SpecTopN=0", "--option1", "UseBook=false", "--option1", "SearchMode=Speculative",
        "--option2", "Threads=1", "--option2", "SpecTopN=0", "--option2", "UseBook=false", "--option2", "SearchMode=Speculative",
        "--initialize-only",
    ]
    subprocess.run(command, check=True, capture_output=True, text=True)
    state = outdir / "state.json"
    assert state.is_file()
    assert json.loads(state.read_text(encoding="utf-8"))["shards"][0]["status"] == "pending"
    state_before_mismatch = state.read_bytes()
    mismatched = command.copy()
    mismatched[mismatched.index("1000", mismatched.index("--byoyomi") + 1)] = "1001"
    rejected = subprocess.run(mismatched, capture_output=True, text=True)
    assert rejected.returncode != 0
    assert "configuration differs" in rejected.stderr
    assert state.read_bytes() == state_before_mismatch
    preflight = root / "preflight.json"
    preflight.write_text("{}", encoding="utf-8")
    execution = outdir / "execution.json"
    subprocess.run([
        sys.executable, str(ROOT / "scripts/record_strength_gate_execution.py"),
        "--plan", str(plan), "--binary", str(engine), "--preflight", str(preflight),
        "--state", str(state), "--output", str(execution),
    ], check=True, capture_output=True, text=True)
    document = json.loads(execution.read_text(encoding="utf-8"))
    assert document["status"] == "running"
    assert document["state_at_recording"]["confirmed_prefix"] == 0
print("PASS")
