#!/usr/bin/env python3
"""Phase 3 validation gate for the king-relative NNUE experiment
(docs/design/nnue_architecture_next_candidate.md, PR #41). Written BEFORE
any Phase 2 training data exists, against the exact rule the user specified
(implementation -> 3-seed training -> validation gate -> fixed-depth sanity
-> paired Elo/SPRT).

Compares architecture B-small (king_relative_b_small) against baseline A
across 3 seeds, using each run's FINAL epoch
(scripts/run_king_relative_phase2.sh's fixed 20-epoch schedule -- no
best-epoch cherry-picking across a run's own history, unlike
select_longrun_checkpoint.py's within-arm epoch selection, which doesn't
apply here since this is an across-architecture comparison at one matched
point, not a search over which epoch of ONE arm to gate).

Primary comparison metric: valid_cp_mse (this project's established
cross-run "common yardstick", per trainer.rs's own ValidStats doc comment
-- comparable across different wdl_lambda, unlike valid_loss). Per seed,
B-small "improves" if its cp_mse is below A's for that same seed.
valid_wdl_loss and valid_calibration_error are reported per seed as
diagnostic context, not part of the pass/fail decision itself -- the user's
checklist asked to "see" them, not to gate on each independently.

Pass bar: B-small improves in >= 2 of 3 seeds, AND no hard-stop condition
fires for ANY seed (checked FIRST, before the seed-count bar, since a
collapsed or badly-saturated run's cp_mse number isn't trustworthy in the
first place):
  - output collapse: valid_output_std under COLLAPSE_STD, or a non-finite
    valid_loss/cp_mse/wdl_loss/calibration_error
  - saturation regression: B-small's l2_dead_neurons more than
    SATURATION_DEAD_NEURON_MARGIN worse than A's, same seed (L2 width is
    unaffected by king_relative_b_small -- only INPUT/L1 differ -- so
    l2_dead_neurons is directly comparable between architectures)

("1 seed wins big alone" / "2 seeds regress", the user's other two named
stop conditions, are exactly what an improved-count < 2 verdict already
looks like -- not separate checks, the natural FAIL case of the seed-count
bar itself.)

Never reads match/game results -- validation-metrics-only by design,
matching select_longrun_checkpoint.py's same prohibition on picking a
checkpoint by looking at playing strength.

Usage: python3 scripts/select_king_relative_checkpoint.py <run_dir> [--epoch N]
  <run_dir>  e.g. data/runs/king_relative_phase2
             (scripts/run_king_relative_phase2.sh's OUT_DIR)
  --epoch N  defaults to 20 (run_king_relative_phase2.sh's EPOCHS)
"""

import json
import math
import sys
from pathlib import Path

ARCHS = ("arch_a", "arch_b")
SEEDS = (42, 7, 123)
DEFAULT_EPOCH = 20

# Same threshold as select_longrun_checkpoint.py's COLLAPSE_STD -- same
# architecture family, same healthy-vs-collapsed output_std range.
COLLAPSE_STD = 5.0

# B-small's l2_dead_neurons may not exceed A's by more than this many
# neurons (same seed) without triggering a stop. Not zero-tolerance: a
# couple of extra dead neurons out of 32 is noise, not a saturation
# regression worth halting the pipeline over.
SATURATION_DEAD_NEURON_MARGIN = 4


def load_meta(run_dir: Path, epoch: int) -> dict:
    """arch -> seed -> meta dict, at the fixed `epoch`."""
    out = {arch: {} for arch in ARCHS}
    for arch in ARCHS:
        for seed in SEEDS:
            path = run_dir / f"{arch}_seed{seed}.epoch{epoch}.meta.json"
            if path.exists():
                out[arch][seed] = json.loads(path.read_text())
    return out


def is_collapsed(m: dict) -> bool:
    for key in ("valid_loss", "valid_cp_mse", "valid_wdl_loss", "valid_calibration_error"):
        v = m.get(key)
        if v is not None and not math.isfinite(v):
            return True
    std = m.get("valid_output_std")
    if std is not None and std < COLLAPSE_STD:
        return True
    return False


def evaluate(meta: dict) -> dict:
    missing = [(arch, seed) for arch in ARCHS for seed in SEEDS if seed not in meta[arch]]
    if missing:
        return {
            "status": "INCOMPLETE_DATA",
            "missing": [f"{arch}_seed{seed}" for arch, seed in missing],
        }

    collapsed = [
        f"{arch}_seed{seed}"
        for arch in ARCHS
        for seed in SEEDS
        if is_collapsed(meta[arch][seed])
    ]
    if collapsed:
        return {"status": "STOP_COLLAPSE", "collapsed": collapsed}

    saturation_regressions = []
    for seed in SEEDS:
        a_dead = meta["arch_a"][seed].get("l2_dead_neurons", 0)
        b_dead = meta["arch_b"][seed].get("l2_dead_neurons", 0)
        if b_dead > a_dead + SATURATION_DEAD_NEURON_MARGIN:
            saturation_regressions.append(
                {
                    "seed": seed,
                    "arch_a_l2_dead_neurons": a_dead,
                    "arch_b_l2_dead_neurons": b_dead,
                }
            )
    if saturation_regressions:
        return {"status": "STOP_SATURATION_REGRESSION", "regressions": saturation_regressions}

    per_seed = {}
    improved_count = 0
    for seed in SEEDS:
        a_cp_mse = meta["arch_a"][seed]["valid_cp_mse"]
        b_cp_mse = meta["arch_b"][seed]["valid_cp_mse"]
        improved = b_cp_mse < a_cp_mse
        improved_count += improved
        per_seed[str(seed)] = {
            "arch_a_cp_mse": a_cp_mse,
            "arch_b_cp_mse": b_cp_mse,
            "b_improved": improved,
            "arch_a_wdl_loss": meta["arch_a"][seed].get("valid_wdl_loss"),
            "arch_b_wdl_loss": meta["arch_b"][seed].get("valid_wdl_loss"),
            "arch_a_calibration_error": meta["arch_a"][seed].get("valid_calibration_error"),
            "arch_b_calibration_error": meta["arch_b"][seed].get("valid_calibration_error"),
        }

    status = "PASS" if improved_count >= 2 else "FAIL_INSUFFICIENT_SEEDS"
    return {
        "status": status,
        "seeds_improved": improved_count,
        "seeds_total": len(SEEDS),
        "per_seed": per_seed,
    }


def _self_check():
    def m(cp_mse, wdl=300.0, calib=0.05, std=50.0, dead=0):
        return {
            "valid_loss": cp_mse,
            "valid_cp_mse": cp_mse,
            "valid_wdl_loss": wdl,
            "valid_calibration_error": calib,
            "valid_output_std": std,
            "l2_dead_neurons": dead,
        }

    # PASS: B-small improves for seeds 42 and 7, loses for 123 -> 2/3.
    meta = {
        "arch_a": {42: m(200), 7: m(210), 123: m(180)},
        "arch_b": {42: m(150), 7: m(190), 123: m(220)},
    }
    r = evaluate(meta)
    assert r["status"] == "PASS", r
    assert r["seeds_improved"] == 2, r

    # FAIL: only 1/3 seeds improve ("1 seed wins big alone" from the user's
    # own framing -- not a separate check, just what this bar naturally
    # rejects).
    meta_fail = {
        "arch_a": {42: m(200), 7: m(210), 123: m(180)},
        "arch_b": {42: m(50), 7: m(400), 123: m(400)},
    }
    r2 = evaluate(meta_fail)
    assert r2["status"] == "FAIL_INSUFFICIENT_SEEDS", r2
    assert r2["seeds_improved"] == 1, r2

    # STOP_COLLAPSE: one seed's output_std collapsed, overrides an
    # otherwise-passing cp_mse comparison.
    meta_collapse = {
        "arch_a": {42: m(200), 7: m(210), 123: m(180)},
        "arch_b": {42: m(150), 7: m(190, std=1.0), 123: m(150)},
    }
    r3 = evaluate(meta_collapse)
    assert r3["status"] == "STOP_COLLAPSE", r3
    assert r3["collapsed"] == ["arch_b_seed7"], r3

    # STOP_SATURATION_REGRESSION: B-small has meaningfully more dead L2
    # neurons than A at the same seed, overrides an otherwise-passing
    # cp_mse comparison.
    meta_sat = {
        "arch_a": {42: m(200, dead=2), 7: m(210, dead=1), 123: m(180, dead=0)},
        "arch_b": {42: m(150, dead=10), 7: m(190, dead=1), 123: m(150, dead=0)},
    }
    r4 = evaluate(meta_sat)
    assert r4["status"] == "STOP_SATURATION_REGRESSION", r4
    assert r4["regressions"][0]["seed"] == 42, r4

    # A small dead-neuron delta (within SATURATION_DEAD_NEURON_MARGIN) must
    # NOT trigger a stop -- noise, not a regression.
    meta_sat_noise = {
        "arch_a": {42: m(200, dead=2), 7: m(210, dead=1), 123: m(180, dead=0)},
        "arch_b": {42: m(150, dead=5), 7: m(190, dead=1), 123: m(150, dead=0)},
    }
    r5 = evaluate(meta_sat_noise)
    assert r5["status"] == "PASS", r5

    # INCOMPLETE_DATA: a missing seed/arch must be reported, not silently
    # treated as 0/2/3.
    meta_incomplete = {
        "arch_a": {42: m(200), 7: m(210)},  # seed 123 missing
        "arch_b": {42: m(150), 7: m(190), 123: m(220)},
    }
    r6 = evaluate(meta_incomplete)
    assert r6["status"] == "INCOMPLETE_DATA", r6
    assert r6["missing"] == ["arch_a_seed123"], r6

    print("self-check ok")


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] == "--self-check":
        _self_check()
        sys.exit(0)
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)
    run_dir = Path(sys.argv[1])
    epoch = DEFAULT_EPOCH
    if "--epoch" in sys.argv:
        epoch = int(sys.argv[sys.argv.index("--epoch") + 1])
    result = evaluate(load_meta(run_dir, epoch))
    print(json.dumps(result, indent=2, ensure_ascii=False))
