#!/usr/bin/env python3
"""Validate and select a completed three-seed self-distillation run.

Each seed first contributes the checkpoint with the lowest blended validation
loss recorded by the trainer.  The selected seed is the median of those three
best losses, avoiding best-of-three cherry-picking.  This is a training-health
selection only; the resulting artifact is not a strength winner until a match
gate passes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import shutil
from pathlib import Path
from typing import Any


SEEDS = (7, 42, 123)
EPOCH_RE = re.compile(r"^Epoch (\d+)/(\d+) —")
VALID_RE = re.compile(r"^  valid: loss=([0-9eE+.-]+)")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def validation_losses(path: Path) -> dict[int, float]:
    losses: dict[int, float] = {}
    epoch: int | None = None
    for raw in path.read_text(encoding="utf-8").splitlines():
        if match := EPOCH_RE.match(raw):
            epoch = int(match.group(1))
            continue
        if match := VALID_RE.match(raw):
            if epoch is None:
                raise ValueError(f"{path}: validation loss precedes epoch header")
            loss = float(match.group(1))
            if not math.isfinite(loss):
                raise ValueError(f"{path}: epoch {epoch} validation loss is not finite")
            losses[epoch] = loss
    return losses


def atomic_copy(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    shutil.copyfile(source, temporary)
    temporary.replace(destination)


def load_seed(run_dir: Path, seed: int) -> dict[str, Any]:
    seed_dir = run_dir / f"seed{seed}"
    final_path = seed_dir / "candidate.bin"
    best_path = seed_dir / "candidate.best.bin"
    log_path = seed_dir / "training.log"
    for required in (final_path, best_path, log_path):
        if not required.is_file():
            raise ValueError(f"seed {seed} is incomplete: missing {required}")

    losses = validation_losses(log_path)
    if not losses:
        raise ValueError(f"seed {seed} has no validation losses")
    best_epoch, best_loss = min(losses.items(), key=lambda item: (item[1], item[0]))
    checkpoint = seed_dir / "checkpoints" / f"candidate.epoch{best_epoch}.bin"
    meta_path = seed_dir / "checkpoints" / f"candidate.epoch{best_epoch}.meta.json"
    if not checkpoint.is_file() or not meta_path.is_file():
        raise ValueError(f"seed {seed} is missing best epoch {best_epoch} artifacts")
    if sha256(checkpoint) != sha256(best_path):
        raise ValueError(f"seed {seed} candidate.best.bin does not match epoch {best_epoch}")

    meta = json.loads(meta_path.read_text(encoding="utf-8"))
    health_fields = (
        "valid_cp_mse",
        "valid_wdl_loss",
        "valid_output_std",
        "valid_output_range",
    )
    if any(not math.isfinite(float(meta.get(key, math.nan))) for key in health_fields):
        raise ValueError(f"seed {seed} best metadata has non-finite health fields")
    healthy = (
        float(meta["valid_output_std"]) >= 5.0
        and float(meta["valid_output_range"]) >= 8.0
        and int(meta.get("l2_dead_neurons", 32)) < 32
        and int(meta.get("cache_misses", 1)) == 0
    )
    return {
        "seed": seed,
        "best_epoch": best_epoch,
        "best_valid_loss": best_loss,
        "best_checkpoint": str(checkpoint),
        "best_checkpoint_sha256": sha256(checkpoint),
        "final_checkpoint": str(final_path),
        "final_checkpoint_sha256": sha256(final_path),
        "metadata": str(meta_path),
        "metadata_sha256": sha256(meta_path),
        "teacher_identity": meta.get("teacher_identity"),
        "dataset_hash": meta.get("dataset_hash"),
        "split_hash": meta.get("split_hash"),
        "label_depth": meta.get("label_depth"),
        "label_time_ms": meta.get("label_time_ms"),
        "label_nodes": meta.get("label_nodes"),
        "valid_cp_mse": meta["valid_cp_mse"],
        "valid_wdl_loss": meta["valid_wdl_loss"],
        "valid_output_std": meta["valid_output_std"],
        "valid_output_range": meta["valid_output_range"],
        "l2_dead_neurons": meta.get("l2_dead_neurons"),
        "cache_hits": meta.get("cache_hits"),
        "cache_misses": meta.get("cache_misses"),
        "training_health": "HEALTHY" if healthy else "WARNING",
    }


def summarize(run_dir: Path, teacher: Path, materialize: bool = True) -> dict[str, Any]:
    cache = run_dir / "teacher_cache_depth2.jsonl"
    if not cache.is_file():
        raise ValueError(f"missing teacher cache: {cache}")
    seeds = [load_seed(run_dir, seed) for seed in SEEDS]
    identities = {item["teacher_identity"] for item in seeds}
    datasets = {item["dataset_hash"] for item in seeds}
    splits = {item["split_hash"] for item in seeds}
    depths = {item["label_depth"] for item in seeds}
    limits = {item["label_time_ms"] for item in seeds}
    node_limits = {item["label_nodes"] for item in seeds}
    if any(
        len(values) != 1
        for values in (identities, datasets, splits, depths, limits, node_limits)
    ):
        raise ValueError("seed runs do not share one teacher/data/split/label contract")
    if depths != {2} or limits != {None} or node_limits != {250000}:
        raise ValueError(
            "expected deterministic depth-2/250000-node labels, "
            f"got depth={depths}, time_limit={limits}, node_limit={node_limits}"
        )

    ordered = sorted(seeds, key=lambda item: (item["best_valid_loss"], item["seed"]))
    selected = ordered[1]
    selected_dir = run_dir / "selected"
    selected_weights = selected_dir / "self_distill_candidate.bin"
    selected_meta = selected_dir / "self_distill_candidate.meta.json"
    if materialize:
        atomic_copy(Path(selected["best_checkpoint"]), selected_weights)
        atomic_copy(Path(selected["metadata"]), selected_meta)

    cache_rows = [line for line in cache.read_text(encoding="utf-8").splitlines() if line.strip()]
    report = {
        "schema": "sekirei.self-distill-selection.v1",
        "selection_rule": (
            "lowest blended valid_loss checkpoint per seed, then median of seeds 7/42/123; "
            "no strength result used"
        ),
        "run_dir": str(run_dir),
        "teacher": {"path": str(teacher), "sha256": sha256(teacher)},
        "teacher_cache": {
            "path": str(cache),
            "sha256": sha256(cache),
            "entries": len(cache_rows),
        },
        "contract": {
            "teacher_identity": next(iter(identities)),
            "dataset_hash": next(iter(datasets)),
            "split_hash": next(iter(splits)),
            "label_depth": next(iter(depths)),
            "label_time_ms": next(iter(limits)),
            "label_nodes": next(iter(node_limits)),
        },
        "seeds": seeds,
        "all_seeds_healthy": all(item["training_health"] == "HEALTHY" for item in seeds),
        "selected_seed": selected["seed"],
        "selected_epoch": selected["best_epoch"],
        "selected_valid_loss": selected["best_valid_loss"],
        "selected_weights": str(selected_weights),
        "selected_weights_sha256": selected["best_checkpoint_sha256"],
        "strength_status": "UNMEASURED",
    }
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--no-materialize", action="store_true")
    args = parser.parse_args()
    try:
        report = summarize(args.run_dir, args.teacher, not args.no_materialize)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    output = args.output or args.run_dir / "selection_manifest.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
