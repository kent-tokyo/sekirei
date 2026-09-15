import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("candidate", ROOT / "scripts/create_fg4_candidate_manifest.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    paths = []
    for name in ("candidate.bin", "baseline.bin", "engine", "corpus.json"):
        path = root / name
        path.write_bytes(name.encode())
        paths.append(path)
    preflight = root / "preflight.json"
    preflight.write_text(json.dumps({"verdict": "REFUSE"}), encoding="utf-8")
    document = MODULE.build(*paths, preflight)
    assert document["status"] == "diagnostic_candidate_not_adopted"
    assert document["preflight"]["verdict"] == "REFUSE"
    assert document["adoption_requirements"]["formal_adoption"] is False
    assert document["claims"]["strength"] == "not_permitted"
    assert document["candidate"]["sha256"] == MODULE.sha256(paths[0])
    validation = root / "validation.json"
    validation.write_text(json.dumps({
        "schema": "sekirei.floodgate-holdout-regression.v1",
        "status": "clean_for_this_diagnostic",
    }), encoding="utf-8")
    validated = MODULE.build(*paths, preflight, validation)
    assert validated["adoption_requirements"]["independent_validation"] == "clean_for_this_diagnostic"
    assert validated["adoption_requirements"]["paired_strength_gate"] == "not_run"
print("PASS")
