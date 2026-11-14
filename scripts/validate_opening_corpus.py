#!/usr/bin/env python3
"""Dispatch exact opening schemas without changing historical replay contracts."""
from pathlib import Path

import freeze_balanced_gate_openings as balanced
import freeze_material_gate_openings as original

DRIVER_PATHS = (Path(__file__), *(Path(__file__).with_name(name) for name in balanced.DRIVER_NAMES))


def validate_corpus(manifest_path: Path, openings_path: Path) -> dict:
    schema = balanced.read_json(manifest_path).get("schema")
    if schema == "sekirei.independent-material-openings.v1":
        return original.validate_corpus(manifest_path, openings_path)
    if schema == balanced.SCHEMA:
        return balanced.validate_corpus(manifest_path, openings_path)
    raise ValueError("unsupported opening corpus schema")
