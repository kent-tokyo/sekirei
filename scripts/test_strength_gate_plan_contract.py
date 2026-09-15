import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


CREATE = load("create_strength_gate_manifest")
VALIDATE = load("validate_strength_gate_manifest")


def write_inputs(root: Path):
    candidate = root / "candidate.bin"
    baseline = root / "baseline.bin"
    calibration = root / "calibration.json"
    openings = root / "openings.sfen"
    candidate.write_bytes(b"candidate")
    baseline.write_bytes(b"baseline")
    for weights in (candidate, baseline):
        value = 14695981039346656037
        for byte in weights.read_bytes():
            value = ((value ^ byte) * 1099511628211) & ((1 << 64) - 1)
        weights.with_suffix(".meta.json").write_text(json.dumps({
            "format": "sekirei-nnue-output-v1",
            "nnue_output": "residual-material",
            "baseline": "material-v1",
            "checkpoint_hash": f"{value:016x}",
        }), encoding="utf-8")
    calibration.write_text(json.dumps({
        "games": 2, "engine1_wins": 1, "draws": 0, "engine2_wins": 1,
        "diversity_ratio": 1.0,
    }), encoding="utf-8")
    openings.write_text("\n".join(
        f"9/9/9/9/4K4/9/9/9/4k4 b - {index + 1}" for index in range(200)
    ) + "\n", encoding="utf-8")
    return candidate, baseline, openings, calibration


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    candidate, baseline, openings, calibration = write_inputs(root)
    output = root / "plan.json"
    import sys
    old_argv = sys.argv
    try:
        sys.argv = [
            "create_strength_gate_manifest.py", "--candidate", str(candidate), "--baseline", str(baseline),
            "--openings", str(openings), "--calibration", str(calibration), "--output", str(output),
            "--candidate-nnue-output", "residual-material",
            "--baseline-nnue-output", "residual-material",
        ]
        CREATE.main()
    finally:
        sys.argv = old_argv
    manifest = json.loads(output.read_text(encoding="utf-8"))
    assert manifest["protocol"]["games_per_position"] == 2
    assert manifest["protocol"]["positions"] == 200
    assert manifest["protocol"]["max_games"] == 400
    assert manifest["protocol"]["sprt"] == {
        "elo0": 0, "elo1": 20, "alpha": 0.05, "beta": 0.05,
        "variant": "trinomial", "paired_by_id": True,
    }
    assert VALIDATE.validate(manifest) == []
    manifest["evaluation"]["candidate"]["mode"] = "absolute"
    assert "evaluation.candidate.sidecar" in VALIDATE.validate(manifest)
    manifest["evaluation"]["candidate"]["mode"] = "residual-material"
    manifest["protocol"]["games_per_position"] = 4
    assert "protocol.games_per_position" in VALIDATE.validate(manifest)
    manifest["protocol"]["games_per_position"] = 2
    manifest["protocol"]["sprt"]["variant"] = "wald"
    assert "protocol.sprt" in VALIDATE.validate(manifest)

print("PASS")
