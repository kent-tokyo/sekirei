import hashlib
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from attach_run_manifest_to_analysis import attach
from validate_analysis_record import validate_lines


def manifest():
    return {
        "schema": "sekirei.csa-run-manifest.v1", "status": "finalized", "engine": "sekirei",
        "engine_version": "0.3.36", "source_revision": "a" * 40, "evaluation": "material",
        "search_backend": "alpha_beta", "hash_mb": 256, "max_depth": 50,
        "resign_score_cp": -2000, "ponder": "disabled", "game_id": "g",
        "server": "localhost", "port": 4081,
        "binary": {"path": "/bin/engine", "bytes": 1, "sha256": "a" * 64},
        "weights": {"path": None, "active": False, "sha256": "b" * 64},
    }


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    manifest_path = root / "manifest.json"
    analysis_path = root / "input.jsonl"
    output_path = root / "output.jsonl"
    manifest_path.write_text(json.dumps(manifest()), encoding="utf-8")
    analysis_path.write_text(
        json.dumps({"schema": "sekirei.analysis-record.v2", "engine": "sekirei",
                    "engine_version": "0.3.36", "score_perspective": "side_to_move",
                    "game_id": "g", "user": "u", "color": "black"}) + "\n"
        + json.dumps({"type": "search", "ply": 0, "side_to_move": "black", "our_color": "black",
                      "sfen": "lnsgkgsnl/1r5b1/p1ppppp1p/7p1/9/9/P1PPPPPP1/1B5R1/LNSGKGSNL b - 1",
                      "bestmove_csa": None, "score_cp": 0, "depth": 1, "nodes": 1,
                      "elapsed_ms": 1, "hashfull": 0, "score_kind": "cp", "bound": "exact",
                      "abort_reason": "none", "pv_csa": None}) + "\n"
        + json.dumps({"type": "game_end", "result": "aborted"}) + "\n",
        encoding="utf-8",
    )
    assert attach(manifest_path, analysis_path, output_path) == 3
    header = json.loads(output_path.read_text(encoding="utf-8").splitlines()[0])
    assert header["run_manifest_sha256"] == hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    assert header["run_manifest_path"] == str(manifest_path)
    assert validate_lines(output_path.read_text(encoding="utf-8").splitlines()) == []
print("PASS")
