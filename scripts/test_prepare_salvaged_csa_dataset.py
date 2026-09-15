#!/usr/bin/env python3
"""Fixture test for deduplicated CSA dataset preparation."""

from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path


SCRIPT = Path(__file__).with_name("prepare_salvaged_csa_dataset.py")
SPEC = importlib.util.spec_from_file_location("prepare_salvaged_csa_dataset", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


with tempfile.TemporaryDirectory() as temp:
    root = Path(temp)
    source = root / "source.csa"
    source.write_text("V2.2\n%TORYO\n", encoding="utf-8")
    manifest = root / "salvage.json"
    manifest.write_text(json.dumps({
        "schema": "sekirei.selfplay-csa-salvage.v1",
        "status": "recovered",
        "games": [
            {"game_number": 1, "usable_for_cp_teacher": True, "split": "train",
             "source": {"path": str(source), "sha256": "example"}},
            {"game_number": 2, "usable_for_cp_teacher": False, "split": "train",
             "source": {"path": str(source)}},
        ],
    }), encoding="utf-8")
    output = root / "derived"
    result = MODULE.prepare(manifest, output)
    assert result["games_total"] == 1
    assert (output / "game0001.csa").is_symlink()
    assert not (output / "game0002.csa").exists()
print("PASS")
