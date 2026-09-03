#!/usr/bin/env python3
"""Create a small, reproducible manifest for a resume verification run."""
import argparse
import hashlib
import json
from pathlib import Path


def record(checkpoint: Path, log: Path, output: Path, dataset: str) -> dict:
    state = json.loads(checkpoint.read_text())
    log_text = log.read_text()
    optimizer = state["optimizer"]
    manifest = {
        "schema": "sekirei.resume-manifest.v1",
        "checkpoint": {
            "path": str(checkpoint),
            "sha256": hashlib.sha256(checkpoint.read_bytes()).hexdigest(),
            "schema": state.get("schema"),
            "epoch_completed": state.get("epoch_completed"),
            "next_game_index": state.get("next_game_index"),
            "config_fingerprint": state.get("config_fingerprint"),
            "optimizer_step": optimizer.get("step"),
            "teacher_cache_entries": len(state.get("teacher_cache", {})),
        },
        "execution": {
            "dataset": dataset,
            "log_path": str(log),
            "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
            "resume_loaded": "resumed complete state from" in log_text,
            "stopped_after_checkpoint": "stopping after requested atomic resume checkpoint" in log_text,
        },
    }
    output.write_text(json.dumps(manifest, indent=2) + "\n")
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--dataset", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    record(args.checkpoint, args.log, args.output, args.dataset)
    print(f"resume manifest written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
