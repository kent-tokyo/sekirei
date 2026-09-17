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
    assert manifest["schema"] == "sekirei.local-selfplay-run.v3"
    assert manifest["status"] == "planned"
    assert manifest["strength_claim"] is False
    assert manifest["options"]["Threads"] == 1
    assert manifest["options"]["SpecTopN"] == 0
    assert manifest["options"]["NnueOutput"] == "absolute"
    assert manifest["command"].count("--engine1") == 1
    assert manifest["positions_count"] == 1
    assert manifest["games_scheduled_expected"] == 2
    assert manifest["collection_policy"]["unique_opening_color_conditions_before_reuse"] == 2

    csa_manifest = output / "csa-manifest.json"
    csa_manifest.write_text(
        json.dumps({"csa_games_written": ["game0001.csa"], "csa_games_skipped": ["game0002"]}),
        encoding="utf-8",
    )
    diagnostics = MODULE.csa_diagnostics(csa_manifest)
    assert diagnostics["csa_games_written"] == ["game0001.csa"]
    assert diagnostics["csa_games_skipped"] == ["game0002"]

    kifu_dir = output / "usi_kifu"
    kifu_dir.mkdir()
    (kifu_dir / "game0001.txt").write_text(
        "# Result: Engine1 Win\nposition startpos moves 7g7f 3c3d\n", encoding="utf-8"
    )
    (kifu_dir / "game0002.txt").write_text(
        "# Result: Engine1 Win\nposition startpos moves 7g7f 3c3d\n", encoding="utf-8"
    )
    dedup = MODULE.write_duplicate_index(kifu_dir, output / "dedup-index.json")
    assert dedup["games_indexed"] == 2
    assert dedup["unique_games"] == 1
    assert dedup["duplicate_games"] == 1
    assert dedup["duplicate_ratio"] == 0.5
    index = json.loads((output / "dedup-index.json").read_text(encoding="utf-8"))
    assert index["groups"][0]["representative"] == "game0001.txt"

    assert MODULE.interrupted_returncode(-2)
    assert MODULE.interrupted_returncode(143)
    assert not MODULE.interrupted_returncode(1)

    audit = {
        "games_reported": 2,
        "usi_kifu_files": 2,
        "record_lines": 0,
    }
    missing = MODULE.missing_required_artifacts(output, audit)
    assert "result.json" in missing
    assert "result.jsonl" in missing
    assert "result.jsonl record for every reported game" in missing

with tempfile.TemporaryDirectory() as temporary:
    temporary_path = Path(temporary)
    fake_engine = temporary_path / "fake-engine"
    fake_engine.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    fake_engine.chmod(0o755)

    incomplete = temporary_path / "incomplete"
    assert (
        MODULE.main(
            [
                "--games",
                "1",
                "--engine",
                str(fake_engine),
                "--runner",
                str(fake_engine),
                "--output",
                str(incomplete),
            ]
        )
        == 1
    )
    incomplete_manifest = json.loads(
        (incomplete / "run-manifest.json").read_text(encoding="utf-8")
    )
    assert incomplete_manifest["status"] == "incomplete"
    assert "result.json" in incomplete_manifest["artifact_audit"]["missing_required_artifacts"]

    interrupted_runner = temporary_path / "interrupted-runner"
    interrupted_runner.write_text("#!/bin/sh\nexit 130\n", encoding="utf-8")
    interrupted_runner.chmod(0o755)
    interrupted = temporary_path / "interrupted"
    assert (
        MODULE.main(
            [
                "--games",
                "1",
                "--engine",
                str(fake_engine),
                "--runner",
                str(interrupted_runner),
                "--output",
                str(interrupted),
            ]
        )
        == 1
    )
    interrupted_manifest = json.loads(
        (interrupted / "run-manifest.json").read_text(encoding="utf-8")
    )
    assert interrupted_manifest["status"] == "interrupted"

with tempfile.TemporaryDirectory() as temporary:
    temporary_path = Path(temporary)
    fake_engine = temporary_path / "fake-engine"
    fake_engine.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    fake_engine.chmod(0o755)
    child_marker = temporary_path / "engine-child-started"
    fake_runner = temporary_path / "fake-runner"
    fake_runner.write_text(
        "#!/bin/sh\n"
        "if [ \"$1\" = \"validate-positions\" ]; then echo '{\"status\":\"valid\",\"positions\":1}'; exit 0; fi\n"
        f"touch '{child_marker}'\n"
        "exit 0\n",
        encoding="utf-8",
    )
    fake_runner.chmod(0o755)
    weak_weights = temporary_path / "weak.bin"
    weak_weights.write_bytes(b"weak")
    rejecting_probe = temporary_path / "rejecting-probe"
    rejecting_probe.write_text("#!/bin/sh\necho '{\"strict\":false}'\nexit 1\n", encoding="utf-8")
    rejecting_probe.chmod(0o755)
    output = temporary_path / "preflight-failed"
    assert MODULE.main([
        "--games", "1", "--engine", str(fake_engine), "--runner", str(fake_runner),
        "--probe", str(rejecting_probe), "--weights", str(weak_weights), "--output", str(output),
    ]) == 1
    manifest = json.loads((output / "run-manifest.json").read_text(encoding="utf-8"))
    assert manifest["status"] == "preflight_failed"
    assert manifest["preflight"]["weights"]["returncode"] == 1
    assert not child_marker.exists()

try:
    MODULE.parse_args(["--games", "0"])
except SystemExit:
    pass
else:
    raise AssertionError("zero games was accepted")

print("PASS")
