#!/usr/bin/env python3
"""Tests for exporting strict NNUE release-gate evidence."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import math
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("gate_evidence", ROOT / "export_nnue_gate_evidence.py")
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def identity(path: Path) -> dict[str, object]:
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def fixture(root: Path, *, threads: int = 1) -> tuple[Path, Path]:
    engine = root / "engine"; engine.write_bytes(b"engine")
    candidate = root / "model.bin"; candidate.write_bytes(b"candidate")
    raw = root / "match.json"
    raw.write_text(json.dumps({
        "engine1_options": {"EvalFile": str(candidate), "FV_SCALE": "24", "NnueOutput": "absolute"},
        "engine2_options": {},
        "engine1_eval_file_acknowledgement": f"info string NNUE weights loaded from {candidate}",
        "engine2_eval_file_acknowledgement": None,
    }), encoding="utf-8")
    upper = math.log(19)
    receipt = root / "result.json"
    receipt.write_text(json.dumps({
        "status": "pass", "terminal_state": "sprt_stopped", "mode": "selfplay",
        "configuration": {"threads": {"a": threads, "b": threads}, "byoyomi_ms": 1000, "fv_scale": 24},
        "result": {"games_played": 40, "wins": 28, "draws": 0, "losses": 12,
                   "sprt": {"variant": "trinomial", "paired_by_position": True, "verdict": "accept_h1",
                            "alpha": 0.05, "beta": 0.05, "elo0": 0.0, "elo1": 20.0,
                            "lower_bound": -upper, "upper_bound": upper, "llr": upper + 0.1,
                            "observations": {"wins": 14, "losses": 6, "draws": 0}}},
        "evidence": {"complete": True,
                     "engines": {"a": identity(engine), "b": identity(engine)},
                     "evaluation": {"mode": "candidate-halfkp-vs-material", "candidate_side": "a",
                                    "baseline": "material", "candidate_nnue_output": "absolute", "file": identity(candidate)},
                     "openings": {"sha256": "1" * 64, "independent_corpus": {}},
                     "outputs": {"raw_match_json": identity(raw)}}
    }), encoding="utf-8")
    return receipt, raw


class ExportGateEvidenceTests(unittest.TestCase):
    def test_exports_verified_candidate_material_gate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result, raw = fixture(root)
            record = MODULE.export(result, raw, "a" * 40)
            self.assertEqual(record["schema"], "sekirei.nnue-engine-gate-evidence.v1")
            self.assertEqual(record["threads"], 1)
            self.assertTrue(record["candidate_load_acknowledged"])
            self.assertEqual(record["sprt"]["analyzed_pairs"], 20)

    def test_rejects_missing_load_acknowledgement(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result, raw = fixture(root)
            document = json.loads(raw.read_text(encoding="utf-8"))
            document["engine1_eval_file_acknowledgement"] = None
            raw.write_text(json.dumps(document), encoding="utf-8")
            receipt = json.loads(result.read_text(encoding="utf-8"))
            receipt["evidence"]["outputs"]["raw_match_json"] = identity(raw)
            result.write_text(json.dumps(receipt), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "missing candidate evaluator acknowledgement"):
                MODULE.export(result, raw, "a" * 40)

    def test_rejects_non_passing_or_incomplete_result(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result, raw = fixture(root)
            document = json.loads(result.read_text(encoding="utf-8"))
            document["status"] = "inconclusive"
            result.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "complete passing"):
                MODULE.export(result, raw, "a" * 40)


if __name__ == "__main__":
    unittest.main()
