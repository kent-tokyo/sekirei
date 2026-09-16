import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("local_selfplay", ROOT / "scripts" / "run_local_selfplay.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


with tempfile.TemporaryDirectory() as temporary:
    output = Path(temporary) / "run"
    assert MODULE.main(["--games", "2", "--output", str(output), "--dry-run"]) == 0
    manifest = json.loads((output / "run-manifest.json").read_text(encoding="utf-8"))
    assert manifest["schema"] == "sekirei.local-selfplay-run.v1"
    assert manifest["status"] == "planned"
    assert manifest["strength_claim"] is False
    assert manifest["options"]["Threads"] == 1
    assert manifest["options"]["SpecTopN"] == 0
    assert manifest["command"].count("--engine1") == 1

    csa_manifest = output / "csa-manifest.json"
    csa_manifest.write_text(
        json.dumps({"csa_games_written": ["game0001.csa"], "csa_games_skipped": ["game0002"]}),
        encoding="utf-8",
    )
    diagnostics = MODULE.csa_diagnostics(csa_manifest)
    assert diagnostics["csa_games_written"] == ["game0001.csa"]
    assert diagnostics["csa_games_skipped"] == ["game0002"]

try:
    MODULE.parse_args(["--games", "0"])
except SystemExit:
    pass
else:
    raise AssertionError("zero games was accepted")

print("PASS")
