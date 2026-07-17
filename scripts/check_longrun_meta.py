#!/usr/bin/env python3
"""Machine verification for the teacher-conflict-masking long-run pipeline
(docs/experiments/teacher_conflict_masking.md's follow-up). Two modes:

  single <meta.json> <expect_conflict_mask: ft|none>
      Checks one run's own .meta.json + checkpoint file for: the mask flag
      recorded as expected, fixed-recipe fields at their intended values,
      a sane (non-0%/non-100%) conflict fire-rate when masking is on, the
      teacher cache actually being reused (not re-searched), no NaN in any
      diagnostic, and checkpoint_hash matching a fresh re-hash of the
      checkpoint file on disk.

  pair <control_meta.json> <candidate_meta.json>
      Checks that a same-seed/same-epoch control/candidate pair differ
      ONLY in mask-related and outcome/diagnostic fields -- every config
      field (dataset, seeds, lr schedule, architecture, ...) must be
      byte-identical.

Exits 0 and prints "OK" (plus details) on success; exits 1 and prints every
failing check on failure. Designed to be run against the smoke test before
committing to the full 6-run x 20-epoch launch, and again against all 36
meta.json files (6 runs x ~6 epochs sampled, or all 20) before the selector.
"""
import hashlib
import json
import math
import sys
from pathlib import Path

FIXED_RECIPE = {
    "wdl_lambda": 0.7,
    "split_seed": 42,
    "shuffle_seed": 11,
    "lr_schedule_epochs": 20,
    "label_depth": 4,
}

# Config fields that must be byte-identical between control and candidate at
# the same seed/epoch -- everything else (diagnostics, valid_*, mask
# bookkeeping, checkpoint_hash, cache_hits/misses) is expected to differ
# because masking changes the training trajectory.
REQUIRED_IDENTICAL_FIELDS = [
    "epoch", "games_dir", "min_rate", "sample", "scored", "label_depth",
    "wdl_lambda", "wdl_target_scale", "phase_weights", "side_balance",
    "source_cap", "validation_ratio", "init_seed", "l2_bias_init",
    "split_seed", "shuffle_seed", "lr", "lr_schedule", "min_lr",
    "warmup_epochs", "epochs", "lr_schedule_epochs", "train_count",
    "valid_count", "train_games", "valid_games", "split_hash",
    "architecture", "git_commit", "dataset_hash", "grad_clip_norm",
    "ft_clip_norm", "l2_clip_norm", "out_clip_norm",
]

DIAG_FIELDS_MUST_BE_PRESENT_AND_FINITE = [
    "ft_active_ratio", "ft_saturation_ratio", "ft_dead_neurons",
    "l2_dead_neurons", "l2_activation_frequency_mean",
    "l2_saturation_frequency_mean", "valid_cp_mse", "valid_wdl_loss",
    "valid_output_mean", "valid_output_std",
]


def fnv1a(data: bytes) -> str:
    h = 14695981039346656037
    mask = (1 << 64) - 1
    for b in data:
        h ^= b
        h = (h * 1099511628211) & mask
    return f"{h:016x}"


def check_single(meta_path: Path, expect_mask: str) -> list[str]:
    errs = []
    m = json.loads(meta_path.read_text())

    mask = m.get("diagnostic_conflict_mask")
    expected = None if expect_mask == "none" else expect_mask
    if mask != expected:
        errs.append(f"diagnostic_conflict_mask = {mask!r}, expected {expected!r}")

    for key, want in FIXED_RECIPE.items():
        got = m.get(key)
        # wdl_lambda round-trips through f32 (0.7 -> 0.699999988...), so
        # floats compare with a tolerance rather than exact equality.
        mismatch = (
            abs(got - want) > 1e-4
            if isinstance(want, float) and isinstance(got, (int, float))
            else got != want
        )
        if mismatch:
            errs.append(f"{key} = {got!r}, expected {want!r}")

    eligible = m.get("eligible_position_count") or 0
    masked = m.get("masked_position_count") or 0
    if expected == "ft":
        if eligible == 0:
            errs.append("eligible_position_count == 0 -- no wdl_target positions seen")
        else:
            rate = masked / eligible
            if rate <= 0.0 or rate >= 1.0:
                errs.append(f"conflict fire-rate = {rate:.1%} -- expected strictly between 0% and 100%")
    else:
        if masked != 0:
            errs.append(f"control run has masked_position_count = {masked}, expected 0")

    hits = m.get("cache_hits") or 0
    misses = m.get("cache_misses") or 0
    total = hits + misses
    if total == 0:
        errs.append("cache_hits + cache_misses == 0 -- teacher cache lookup never happened")
    else:
        hit_rate = hits / total
        if hit_rate < 0.99:
            errs.append(
                f"teacher cache hit rate = {hit_rate:.1%} ({hits} hits / {misses} misses) "
                "-- expected >=99% given the cache was pre-populated from prior runs"
            )

    for key in DIAG_FIELDS_MUST_BE_PRESENT_AND_FINITE:
        v = m.get(key)
        if v is None:
            errs.append(f"{key} missing from meta.json")
        elif isinstance(v, (int, float)) and not math.isfinite(v):
            errs.append(f"{key} = {v} -- not finite")

    if (m.get("valid_output_std") or 0.0) < 5.0:
        errs.append(f"valid_output_std = {m.get('valid_output_std')} -- looks collapsed (<5.0)")

    ckpt_path = meta_path.with_suffix("").with_suffix(".bin")
    if not ckpt_path.exists():
        errs.append(f"checkpoint file not found: {ckpt_path}")
    else:
        actual_hash = fnv1a(ckpt_path.read_bytes())
        recorded_hash = m.get("checkpoint_hash")
        if actual_hash != recorded_hash:
            errs.append(
                f"checkpoint_hash mismatch: recorded {recorded_hash}, "
                f"re-hash of {ckpt_path} gives {actual_hash}"
            )

    return errs


def _fields_differ(a, b) -> bool:
    if isinstance(a, float) and isinstance(b, (int, float)):
        return abs(a - b) > 1e-4
    return a != b


def check_pair(control_path: Path, candidate_path: Path) -> list[str]:
    errs = []
    c = json.loads(control_path.read_text())
    d = json.loads(candidate_path.read_text())
    for key in REQUIRED_IDENTICAL_FIELDS:
        if _fields_differ(c.get(key), d.get(key)):
            errs.append(f"{key} differs: control={c.get(key)!r} candidate={d.get(key)!r}")
    if d.get("diagnostic_conflict_mask") is None:
        errs.append("candidate has diagnostic_conflict_mask=null -- not actually masking")
    if c.get("diagnostic_conflict_mask") is not None:
        errs.append(f"control has diagnostic_conflict_mask={c.get('diagnostic_conflict_mask')!r} -- should be null")
    return errs


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "single":
        errors = check_single(Path(sys.argv[2]), sys.argv[3])
    elif len(sys.argv) == 4 and sys.argv[1] == "pair":
        errors = check_pair(Path(sys.argv[2]), Path(sys.argv[3]))
    else:
        print(__doc__)
        sys.exit(1)

    if errors:
        print(f"FAIL ({len(errors)} check(s) failed):")
        for e in errors:
            print(f"  - {e}")
        sys.exit(1)
    print("OK")
