#!/usr/bin/env python3
"""Contract tests for the separately distributed NNUE artifact validator."""
from __future__ import annotations

import hashlib
import importlib.util
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("artifact", ROOT / "validate_nnue_release_artifact.py")
assert SPEC and SPEC.loader
ARTIFACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ARTIFACT)


def card(payload: bytes) -> dict:
    return {
        "schema": "sekirei.nnue-model-card.v1", "artifact": "weights/model.bin",
        "sha256": hashlib.sha256(payload).hexdigest(), "bytes": len(payload),
        "architecture": "A-flat-ps", "nnue_output": "absolute", "license": "CC-BY-4.0",
        "attribution": "Sekirei project, Kentaro Tanabe", "training": {"teacher": {}},
        "current_status": {
            "availability": "published_optional_checkpoint",
            "recommendation": "hold_after_current_material_comparison",
            "reason": "Current material comparison is complete.",
            "comparison": {
                "status": "complete", "pooled": False,
                "settings": [{"byoyomi_ms": 1000}, {"byoyomi_ms": 5000}],
            },
        },
        "strength_gate": {
            "verdict": "PASS", "games": 94,
            "scope": "Local only; not a Floodgate rating.",
            "baseline": {"path": "baseline.bin", "sha256": "0" * 64},
        },
    }


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        target = root / "weights"
        target.mkdir()
        payload = b"SEKIRW01" + b"fixture"
        (target / "model.bin").write_bytes(payload)
        document = card(payload)
        assert ARTIFACT.validate(document, root) == []
        document["bytes"] += 1
        assert "bytes" in ARTIFACT.validate(document, root)
        document["bytes"] -= 1
        document["current_status"]["recommendation"] = "recommended"
        assert "current_status.contract" in ARTIFACT.validate(document, root)
    print("NNUE release artifact validator: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
