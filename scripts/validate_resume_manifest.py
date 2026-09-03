#!/usr/bin/env python3
"""Validate resume-run lineage manifests without running training."""
import json
import re
import sys
from pathlib import Path

HEX64 = re.compile(r"^[0-9a-f]{64}$")


def validate(doc):
    errors = []
    if doc.get("schema") != "sekirei.resume-manifest.v1":
        errors.append("schema")
    checkpoint = doc.get("checkpoint", {})
    if not isinstance(checkpoint.get("path"), str):
        errors.append("checkpoint.path")
    if not HEX64.fullmatch(checkpoint.get("sha256", "")):
        errors.append("checkpoint.sha256")
    if checkpoint.get("schema") != "sekirei.resume-checkpoint.v1":
        errors.append("checkpoint.schema")
    if not isinstance(checkpoint.get("epoch_completed"), int) or checkpoint["epoch_completed"] < 0:
        errors.append("checkpoint.epoch_completed")
    if not isinstance(checkpoint.get("next_game_index"), int) or checkpoint["next_game_index"] < 0:
        errors.append("checkpoint.next_game_index")
    if not isinstance(checkpoint.get("config_fingerprint"), str) or not checkpoint["config_fingerprint"]:
        errors.append("checkpoint.config_fingerprint")
    if not isinstance(checkpoint.get("optimizer_step"), int) or checkpoint["optimizer_step"] < 0:
        errors.append("checkpoint.optimizer_step")
    if not isinstance(checkpoint.get("teacher_cache_entries"), int) or checkpoint["teacher_cache_entries"] < 0:
        errors.append("checkpoint.teacher_cache_entries")
    execution = doc.get("execution", {})
    if not isinstance(execution.get("dataset"), str) or not execution["dataset"]:
        errors.append("execution.dataset")
    if not isinstance(execution.get("log_path"), str):
        errors.append("execution.log_path")
    if not HEX64.fullmatch(execution.get("log_sha256", "")):
        errors.append("execution.log_sha256")
    for key in ("resume_loaded", "stopped_after_checkpoint"):
        if not isinstance(execution.get(key), bool):
            errors.append(f"execution.{key}")
    return errors


def main(argv=None):
    args = argv or sys.argv[1:]
    if len(args) != 1:
        print("usage: validate_resume_manifest.py MANIFEST", file=sys.stderr)
        return 2
    path = Path(args[0])
    try:
        doc = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        print(f"invalid resume manifest: {exc}", file=sys.stderr)
        return 2
    errors = validate(doc)
    if errors:
        print("invalid resume manifest: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid resume manifest: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
