"""Contract tests for the Q30-reduced Q27 preparation and finalizer."""

from __future__ import annotations

import argparse
import importlib.util
import json
import tempfile
from pathlib import Path

from validate_nnue_output_metadata import checkpoint_hash


ROOT = Path(__file__).resolve().parents[1]


def module(name: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    loaded = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(loaded)
    return loaded


PREPARE = module("prepare_q27_q30_match")
FINALIZE = module("finalize_q27_q30_match")


def write(path: Path, text: str = "x") -> Path:
    path.write_text(text, encoding="utf-8")
    return path


def write_json(path: Path, value: object) -> Path:
    path.write_text(json.dumps(value, sort_keys=True), encoding="utf-8")
    return path


def candidate(path: Path) -> Path:
    path.write_bytes(b"q30-reduced-candidate")
    write_json(path.with_suffix(".meta.json"), {
        "format": "sekirei-nnue-output-v1", "nnue_output": "residual-material",
        "baseline": "material-v1", "checkpoint_hash": checkpoint_hash(path),
    })
    return path


def prepared(directory: Path):
    candidate_file = candidate(directory / "candidate.bin")
    candidate_engine = write(directory / "candidate-engine")
    material_engine = write(directory / "material-engine")
    runner = write(directory / "runner")
    finalizer = write(directory / "finalizer")
    q30 = write_json(directory / "q30.json", {
        "schema": "sekirei.q30-efficiency-validation-decision.v1", "status": "pass",
        "q27_authorized": True, "q20_authorized": False,
        "artifacts": {"reduced_candidate": {"sha256": PREPARE.sha256(candidate_file)}},
    })
    preflight = write_json(directory / "preflight.json", {
        "schema": "sekirei.gate-resource-preflight.v1", "mode": "formal_preflight",
        "formal_measurement_eligible": True, "verdict": "pass",
    })
    openings = write(directory / "openings.sfen", "\n".join(f"sfen-{index}" for index in range(16)) + "\n")
    excluded = write_json(directory / "excluded.json", {"positions": []})
    manifest = write_json(directory / "openings.json", {
        "schema": "sekirei.gate-opening-corpus.v1", "positions": 16,
        "openings_sha256": PREPARE.sha256(openings),
        "excluded_source_manifests": [{"path": str(excluded), "sha256": PREPARE.sha256(excluded)}],
    })
    args = argparse.Namespace(
        q30_screen_decision=q30, preflight=preflight, candidate_engine=candidate_engine,
        material_engine=material_engine, runner=runner, candidate=candidate_file,
        openings=openings, openings_manifest=manifest, finalizer=finalizer,
        required_exclusion_manifest=[excluded], output=directory / "plan.json",
    )
    plan = PREPARE.prepare(args)
    write_json(args.output, plan)
    return args, plan


def test_prepare_binds_q30_pass_and_rejects_refused_preflight():
    with tempfile.TemporaryDirectory() as raw:
        args, plan = prepared(Path(raw))
        assert plan["protocol"]["games"] == 32
        assert plan["runtime"]["candidate"]["sha256"] == PREPARE.sha256(args.candidate)
        refused = json.loads(args.preflight.read_text(encoding="utf-8"))
        refused["verdict"] = "refuse"
        write_json(args.preflight, refused)
        try:
            PREPARE.prepare(args)
        except ValueError as error:
            assert "preflight" in str(error)
        else:
            raise AssertionError("refused formal preflight was accepted")


def test_finalizer_requires_complete_pairs_and_authorizes_q20_only_at_60pct():
    with tempfile.TemporaryDirectory() as raw:
        directory = Path(raw)
        args, plan = prepared(directory)
        result = {
            "status": "complete", "games": 32, "engine1_wins": 20, "draws": 0, "engine2_wins": 12,
            "invalid_games": [], "artifact_write_failures": [],
            "engine1_options": {**plan["protocol"]["engine_options"], **plan["protocol"]["candidate_options"]},
            "engine2_options": dict(plan["protocol"]["engine_options"]),
            "engine1_eval_file_acknowledgement": f"info string NNUE weights loaded from {args.candidate}",
            "engine2_eval_file_acknowledgement": None,
        }
        result_path = write_json(directory / "result.json", result)
        records = []
        for index in range(16):
            outcome = "candidate_win" if index < 10 else "baseline_win"
            records.extend({"id": f"opening-{index}", "result": outcome} for _ in range(2))
        records_path = write(directory / "result.jsonl", "\n".join(json.dumps(row) for row in records) + "\n")
        csa_path = write_json(directory / "csa.json", {"status": "complete", "games_completed": 32})
        decision = FINALIZE.finalize(args.output, result_path, records_path, csa_path)
        assert decision["result"]["verdict"] == "PASS_TO_Q20"
        assert decision["decision"]["q20_authorized"] is True
