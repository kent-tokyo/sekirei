#!/usr/bin/env python3
"""Attach verified resume evidence to a release-manifest copy."""
import argparse
import json
from pathlib import Path

from validate_release_manifest import validate as validate_release
from validate_resume_manifest import validate as validate_resume


def attach(release_path: Path, resume_path: Path, output_path: Path) -> dict:
    release = json.loads(release_path.read_text())
    resume = json.loads(resume_path.read_text())
    release_errors = validate_release(release)
    resume_errors = validate_resume(resume)
    if release_errors:
        raise ValueError("invalid release manifest: " + ", ".join(release_errors))
    if resume_errors:
        raise ValueError("invalid resume manifest: " + ", ".join(resume_errors))
    checkpoint = resume["checkpoint"]
    execution = resume["execution"]
    release["resume_verification"] = {
        "schema": "sekirei.resume-manifest.v1",
        "status": "verified",
        "checkpoint_path": checkpoint["path"],
        "checkpoint_sha256": checkpoint["sha256"],
        "log_path": execution["log_path"],
        "log_sha256": execution["log_sha256"],
        "epoch_completed": checkpoint["epoch_completed"],
        "next_game_index": checkpoint["next_game_index"],
        "config_fingerprint": checkpoint["config_fingerprint"],
        "optimizer_step": checkpoint["optimizer_step"],
        "teacher_cache_entries": checkpoint["teacher_cache_entries"],
        "artifacts": [
            {"kind": "resume_checkpoint", "path": checkpoint["path"], "sha256": checkpoint["sha256"]},
            {"kind": "execution_log", "path": execution["log_path"], "sha256": execution["log_sha256"]},
        ],
    }
    output_path.write_text(json.dumps(release, indent=2) + "\n")
    return release


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--release-manifest", type=Path, required=True)
    parser.add_argument("--resume-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        attach(args.release_manifest, args.resume_manifest, args.output)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        parser.error(str(exc))
    print(f"release manifest copy written: {args.output}")


if __name__ == "__main__":
    main()
