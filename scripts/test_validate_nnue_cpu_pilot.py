#!/usr/bin/env python3
"""Focused contract tests for the CPU-first NNUE pilot validator."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("pilot", ROOT / "validate_nnue_cpu_pilot.py")
assert SPEC and SPEC.loader
PILOT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PILOT)


def reference(root: Path, relative: str) -> dict[str, str]:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = relative.encode("utf-8")
    path.write_bytes(payload)
    return {"path": relative, "sha256": hashlib.sha256(payload).hexdigest()}


def valid_plan(root: Path) -> dict:
    evidence = {name: reference(root, f"evidence/{name}.json") for name in PILOT.REQUIRED_REFERENCES}
    evidence["bottleneck"] = {
        "code_correctness": "resolved",
        "teacher_quality": "resolved",
        "model_design": "resolved",
        "cpu_dominant_phase": "label_generation",
        "local_cpu_seconds": 900,
        "local_wall_seconds": 1000,
    }
    return {
        "schema": PILOT.SCHEMA,
        "candidate": {
            "id": "fixture-nnue",
            "sha256": "a" * 64,
            "engine_commit": "b" * 40,
            "evaluation": "candidate-vs-material",
            "paired_colour_reversed": True,
            "material_gate_verdict": "PASS",
        },
        "evidence": evidence,
        "pilot": {
            "provider": "aws-ec2",
            "instance_type": "c7i.xlarge",
            "vcpus": 4,
            "estimated_minutes": 90,
            "max_minutes": 120,
            "workload_positions": 200000,
            "metrics": sorted(PILOT.REQUIRED_METRICS),
            "stop_conditions": sorted(PILOT.REQUIRED_STOP_CONDITIONS),
        },
    }


class CpuPilotTests(unittest.TestCase):
    def test_accepts_fully_bound_short_cpu_pilot(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(PILOT.validate(valid_plan(root), root), [])

    def test_rejects_unqualified_candidate_and_unresolved_bottleneck(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = valid_plan(root)
            plan["candidate"]["material_gate_verdict"] = "INCONCLUSIVE"
            plan["evidence"]["bottleneck"]["teacher_quality"] = "unknown"
            errors = PILOT.validate(plan, root)
            self.assertIn("candidate.material_gate_verdict", errors)
            self.assertIn("evidence.bottleneck.teacher_quality", errors)

    def test_rejects_missing_integrity_and_stop_contracts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = valid_plan(root)
            plan["evidence"]["checkpoint_resume_manifest"]["sha256"] = "0" * 64
            plan["pilot"]["metrics"].remove("resume_verified")
            plan["pilot"]["stop_conditions"].remove("hash_mismatch")
            errors = PILOT.validate(plan, root)
            self.assertIn("evidence.checkpoint_resume_manifest.sha256", errors)
            self.assertIn("pilot.metrics", errors)
            self.assertIn("pilot.stop_conditions", errors)

    def test_rejects_malformed_metric_entries_without_crashing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = valid_plan(root)
            plan["pilot"]["metrics"].append({"not": "a metric"})
            self.assertIn("pilot.metrics", PILOT.validate(plan, root))


if __name__ == "__main__":
    unittest.main()
