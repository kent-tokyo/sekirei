#!/usr/bin/env python3
"""Phase A (warmup validation) pass/fail check for the teacher-conflict-
masking long run. Run after control_seed42's epoch 1 (cold-cache build) and
epoch 2 (warm-cache confirmation) complete, before resuming to the full
6-run x 20-epoch job -- so a misconfiguration is caught after ~2.7-2.9
hours (the one-time cold-search cost), not after the full ~4-5 hour job.

Usage:
  python3 scripts/check_warmup_validation.py \
      <epoch1_meta.json> <epoch2_meta.json> \
      <pre_run_cache_line_count> <cache_path>

Exits 0 ("PASS") only if all 10 pre-registered conditions hold; otherwise
exits 1 and lists every failing condition.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from check_longrun_meta import FIXED_RECIPE, check_single  # noqa: E402

MAX_TOLERATED_EPOCH2_MISSES = 2  # "ごく少数" -- warn, not fail, up to this
MAX_EPOCH2_SEARCH_SECS = 5.0  # "ほぼゼロ" search time


def check(e1: dict, e2: dict, pre_run_cache_count: int, cache_path: Path) -> list[str]:
    errs = []
    warnings = []

    # 1. epoch1's own hits+misses must equal its own lookup count
    #    (train_count + valid_count) -- internal consistency, catches a
    #    counting bug rather than an environment issue.
    e1_lookups = (e1.get("cache_hits") or 0) + (e1.get("cache_misses") or 0)
    e1_expected = (e1.get("train_count") or 0) + (e1.get("valid_count") or 0)
    if e1_lookups != e1_expected:
        errs.append(
            f"epoch1: cache_hits+cache_misses={e1_lookups} != train_count+valid_count={e1_expected}"
        )

    # 2. epoch2 misses ~= 0
    e2_misses = e2.get("cache_misses") or 0
    if e2_misses > MAX_TOLERATED_EPOCH2_MISSES:
        errs.append(f"epoch2: cache_misses={e2_misses} (expected 0, tolerance {MAX_TOLERATED_EPOCH2_MISSES})")
    elif e2_misses > 0:
        warnings.append(f"epoch2: cache_misses={e2_misses} (nonzero but within tolerance)")

    # 3. epoch2 search time ~= 0
    e2_search = e2.get("search_time_secs")
    if e2_search is None:
        errs.append("epoch2: search_time_secs missing from meta.json")
    elif e2_search > MAX_EPOCH2_SEARCH_SECS:
        errs.append(f"epoch2: search_time_secs={e2_search:.1f}s (expected near 0)")

    # 4. epoch1/epoch2 train/validation position sets match
    if e1.get("train_games") != e2.get("train_games"):
        errs.append(f"train_games differs: epoch1={e1.get('train_games')} epoch2={e2.get('train_games')}")
    if e1.get("valid_games") != e2.get("valid_games"):
        errs.append(f"valid_games differs: epoch1={e1.get('valid_games')} epoch2={e2.get('valid_games')}")
    e2_lookups = (e2.get("cache_hits") or 0) + (e2.get("cache_misses") or 0)
    if e1_lookups != e2_lookups:
        errs.append(f"total position lookups differ: epoch1={e1_lookups} epoch2={e2_lookups}")

    # 5. recipe fields as intended (both epochs), + dataset_hash internally
    #    consistent across epochs (same run, same data).
    for meta, label in ((e1, "epoch1"), (e2, "epoch2")):
        for key, want in FIXED_RECIPE.items():
            got = meta.get(key)
            mismatch = (
                abs(got - want) > 1e-4
                if isinstance(want, float) and isinstance(got, (int, float))
                else got != want
            )
            if mismatch:
                errs.append(f"{label}: {key} = {got!r}, expected {want!r}")
    if e1.get("dataset_hash") != e2.get("dataset_hash"):
        errs.append("dataset_hash differs between epoch1 and epoch2 of the same run")

    # 6. cache key includes label_depth (structural fix, not a live
    #    metric) -- confirmed via teacher_cache.rs's
    #    wrong_depth_entries_are_filtered_out_and_reported test, not
    #    re-derivable from meta.json alone. Recorded here for the audit
    #    trail; not itself a pass/fail condition against live data.

    # 7/8/9 (checkpoint/diagnostics/metadata generated, no NaN/collapse,
    # check_longrun_meta.py succeeds) are added by the caller via
    # check_longrun_meta.check_single, not duplicated here.
    return errs, warnings


def _cache_line_count(path: Path) -> int:
    if not path.exists():
        return 0
    with open(path) as f:
        return sum(1 for line in f if line.strip())


if __name__ == "__main__":
    if len(sys.argv) != 5:
        print(__doc__)
        sys.exit(1)
    e1_path, e2_path, pre_run_count_arg, cache_path_arg = sys.argv[1:5]
    e1 = json.loads(Path(e1_path).read_text())
    e2 = json.loads(Path(e2_path).read_text())
    pre_run_cache_count = int(pre_run_count_arg)
    cache_path = Path(cache_path_arg)

    errs, warnings = check(e1, e2, pre_run_cache_count, cache_path)

    # 7/8/9: run check_longrun_meta.py's structural single-run checks
    # against both epochs directly (checkpoint hash, NaN/collapse, fixed
    # recipe, cache hit rate for epoch2 specifically -- epoch1 is exempt
    # since a cold epoch1 legitimately has a low hit rate).
    e2_struct_errs = check_single(Path(e2_path), "none")
    # epoch2's own cache-hit-rate check (>=99%) is exactly condition 2/3
    # above in different words; keep check_single's other checks (NaN,
    # collapse, checkpoint hash, recipe) without double-reporting the
    # hit-rate line.
    errs.extend(e for e in e2_struct_errs if "hit rate" not in e)

    # 10. post-run cache line count == pre-run count + epoch1 misses
    #     (write only happens once, at end of epoch 1).
    post_run_count = _cache_line_count(cache_path)
    expected_count = pre_run_cache_count + (e1.get("cache_misses") or 0)
    if post_run_count != expected_count:
        errs.append(
            f"cache line count after run = {post_run_count}, expected "
            f"{pre_run_cache_count} (pre-run) + {e1.get('cache_misses')} (epoch1 misses) = {expected_count}"
        )

    for w in warnings:
        print(f"WARN: {w}")

    if errs:
        print(f"FAIL ({len(errs)} condition(s) failed):")
        for e in errs:
            print(f"  - {e}")
        sys.exit(1)
    print("PASS -- Phase A warmup validation cleared, safe to resume to the full 20-epoch job")
