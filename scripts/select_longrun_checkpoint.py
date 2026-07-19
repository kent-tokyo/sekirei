#!/usr/bin/env python3
"""Pre-registered checkpoint selection for the teacher-conflict-masking long
run (docs/experiments/teacher_conflict_masking.md's follow-up). Written
BEFORE any 20-epoch data exists, against the exact rule the user specified:

  1. Exclude any epoch where ANY of the 3 conflict_ft seeds shows collapse
     (NaN, or output_std collapsed, or L2 fully dead).
  2. Among surviving epochs, pick the one with minimum 3-seed-average
     valid_cp_mse for conflict_ft.
  3. At that epoch, if 3-seed-average valid_wdl_loss for conflict_ft is
     worse than control's by more than WDL_NONINFERIORITY_TOL (relative) --
     stop, do not gate.
  4. Ties in step 2 (within TIE_TOL relative) resolve to the earlier epoch.
  5. The seed used for the paired gate is whichever of the 3 conflict_ft
     seeds has the MEDIAN valid_cp_mse at the selected epoch. Control uses
     that same seed and same epoch.
  6. This selection runs exactly ONCE, before any match/game result exists.
     The output is never revisited after seeing a gate result -- a gate
     FAIL means "masking not adopted," not "go pick a different epoch/seed
     and re-gate."

Selection is validation-metrics-only by design -- this script never reads
match/game results, matching the user's explicit prohibition on picking a
checkpoint by looking at playing strength.

Usage: python3 scripts/select_longrun_checkpoint.py <run_dir>
"""
import json
import math
import statistics
import sys
from pathlib import Path

ARMS = ("control", "conflict_ft")
SEEDS = (42, 7, 123)
TIE_TOL = 0.001  # 0.1% relative -- ties resolve to the earlier epoch

# A checkpoint counts as "collapsed" (pre-registered, before seeing data):
#   - any of valid_cp_mse / valid_wdl_loss / valid_output_std is NaN or inf
#   - valid_output_std < COLLAPSE_STD (near-frozen output; the short-run
#     experiment's healthy range was ~30-80, its collapsed runs were ~2-4)
#   - l2_dead_neurons == L2 -- every L2 neuron dead
COLLAPSE_STD = 5.0
# Corrected 2026-07-20: the actual architecture (every longrun .meta.json's
# "architecture" field, and the 32-entry l2_saturation_frequency_per_neuron
# arrays) is L1=256 L2=32, not 16. This was wrong from this script's
# introduction through the teacher-conflict-masking gate; see tasks/lessons.md
# 2026-07-20 entry for the correction and verified blast radius (none, for
# that gate -- no epoch in data/runs/20260717_longrun_conflict_mask ever had
# l2_dead_neurons in [16, 31], so this bug never actually changed which
# epoch/seed the pre-registered rule selected).
L2 = 32  # sekirei_core::nnue::L2 -- keep in sync if the architecture changes

# valid_wdl_loss "non-inferior" means within this relative margin of
# control's -- not a strict >. Guards against a spurious STOP triggered by
# float/seed noise rather than a real regression. 0.5% is deliberately
# tight: the actual seed-42 regression observed in the short-run experiment
# was ~0.92% (352055 -> 355301), comfortably outside this margin, so a real
# regression of that size still stops the pipeline; only sub-0.5% wobble is
# forgiven.
WDL_NONINFERIORITY_TOL = 0.005


def load_meta(run_dir: Path) -> dict:
    """arm -> seed -> epoch -> meta dict"""
    out = {arm: {seed: {} for seed in SEEDS} for arm in ARMS}
    for meta_path in run_dir.glob("*.meta.json"):
        # "<arm>_seed<seed>.epoch<epoch>.meta.json"
        stem = meta_path.name.removesuffix(".meta.json")
        arm_seed, epoch_part = stem.rsplit(".epoch", 1)
        arm, seed_part = arm_seed.rsplit("_seed", 1)
        seed, epoch = int(seed_part), int(epoch_part)
        if arm not in out or seed not in out[arm]:
            continue
        out[arm][seed][epoch] = json.loads(meta_path.read_text())
    return out


def is_collapsed(m: dict) -> bool:
    for key in ("valid_cp_mse", "valid_wdl_loss", "valid_output_std"):
        v = m.get(key)
        if v is None or not math.isfinite(v):
            return True
    if m["valid_output_std"] < COLLAPSE_STD:
        return True
    if m.get("l2_dead_neurons", 0) >= L2:
        return True
    return False


def select(meta: dict) -> dict:
    conflict = meta["conflict_ft"]
    control = meta["control"]
    epochs = sorted(set.intersection(*(set(conflict[s]) for s in SEEDS)))
    if not epochs:
        return {"status": "NO_DATA"}

    excluded = {
        e for e in epochs if any(is_collapsed(conflict[s][e]) for s in SEEDS)
    }
    candidates = [e for e in epochs if e not in excluded]
    if not candidates:
        return {"status": "ALL_EPOCHS_EXCLUDED", "excluded_epochs": sorted(excluded)}

    avg_cp_mse = {
        e: statistics.mean(conflict[s][e]["valid_cp_mse"] for s in SEEDS)
        for e in candidates
    }
    best = min(avg_cp_mse.values())
    tied = sorted(e for e, v in avg_cp_mse.items() if v <= best * (1 + TIE_TOL))
    selected_epoch = tied[0]  # earliest among ties

    conflict_wdl = statistics.mean(
        conflict[s][selected_epoch]["valid_wdl_loss"] for s in SEEDS
    )
    control_wdl = statistics.mean(
        control[s][selected_epoch]["valid_wdl_loss"] for s in SEEDS
    )
    if conflict_wdl > control_wdl * (1 + WDL_NONINFERIORITY_TOL):
        return {
            "status": "STOP_WDL_REGRESSION",
            "selected_epoch": selected_epoch,
            "conflict_wdl_3seed_avg": conflict_wdl,
            "control_wdl_3seed_avg": control_wdl,
            "wdl_relative_regression": conflict_wdl / control_wdl - 1,
            "excluded_epochs": sorted(excluded),
        }

    seed_cp_mse = {s: conflict[s][selected_epoch]["valid_cp_mse"] for s in SEEDS}
    median_val = statistics.median(seed_cp_mse.values())
    gate_seed = min(seed_cp_mse, key=lambda s: abs(seed_cp_mse[s] - median_val))

    return {
        "status": "PROCEED_TO_GATE",
        "selected_epoch": selected_epoch,
        "gate_seed": gate_seed,
        "candidate_checkpoint": f"conflict_ft_seed{gate_seed}.epoch{selected_epoch}.bin",
        "baseline_checkpoint": f"control_seed{gate_seed}.epoch{selected_epoch}.bin",
        "conflict_cp_mse_3seed_avg": avg_cp_mse[selected_epoch],
        "conflict_wdl_3seed_avg": conflict_wdl,
        "control_wdl_3seed_avg": control_wdl,
        "seed_cp_mse_at_selected_epoch": seed_cp_mse,
        "excluded_epochs": sorted(excluded),
    }


def _self_check():
    def m(cp_mse, wdl, std=50.0, dead=0):
        return {
            "valid_cp_mse": cp_mse,
            "valid_wdl_loss": wdl,
            "valid_output_std": std,
            "l2_dead_neurons": dead,
        }

    meta = {
        "conflict_ft": {
            42: {1: m(200, 300), 2: m(150, 310), 3: m(180, 320)},
            7: {1: m(210, 290), 2: m(140, 280), 3: m(190, 300)},
            123: {1: m(220, 295), 2: m(130, 285), 3: m(170, 290)},
        },
        "control": {
            42: {1: m(250, 300), 2: m(200, 300), 3: m(220, 300)},
            7: {1: m(260, 300), 2: m(210, 300), 3: m(230, 300)},
            123: {1: m(255, 300), 2: m(205, 300), 3: m(225, 300)},
        },
    }
    r = select(meta)
    assert r["status"] == "PROCEED_TO_GATE", r
    assert r["selected_epoch"] == 2, r  # min 3-seed-avg cp_mse is epoch 2
    assert r["gate_seed"] == 7, r  # cp_mse at epoch 2: 42->150, 7->140, 123->130; median=140->seed 7

    meta["conflict_ft"][7][2] = m(math.nan, 280)
    r2 = select(meta)
    assert r2["excluded_epochs"] == [2], r2  # epoch 2 excluded by seed 7's NaN
    assert r2.get("selected_epoch") != 2, r2

    # STOP_WDL_REGRESSION fires when the selected epoch's 3-seed-avg WDL
    # loss is worse than control's, even if CP MSE improved.
    meta2 = {
        "conflict_ft": {s: {1: m(100, 400)} for s in SEEDS},
        "control": {s: {1: m(200, 300)} for s in SEEDS},
    }
    r3 = select(meta2)
    assert r3["status"] == "STOP_WDL_REGRESSION", r3

    # A sub-tolerance WDL "regression" (0.2% here, under the 0.5% margin)
    # must NOT stop the pipeline.
    meta3 = {
        "conflict_ft": {s: {1: m(100, 300.5)} for s in SEEDS},
        "control": {s: {1: m(200, 300.0)} for s in SEEDS},
    }
    r4 = select(meta3)
    assert r4["status"] == "PROCEED_TO_GATE", r4

    # Regression guard for the 2026-07-20 L2=16->32 fix: is_collapsed must
    # trigger at the real L2 width (32 dead), not the old wrong one (16
    # dead). 16 dead out of 32 neurons is a real, unhealthy signal but must
    # NOT be treated as full collapse.
    assert is_collapsed(m(100, 300, dead=32)) is True
    assert is_collapsed(m(100, 300, dead=31)) is False
    assert is_collapsed(m(100, 300, dead=16)) is False

    print("self-check ok")


if __name__ == "__main__":
    if len(sys.argv) == 2 and sys.argv[1] == "--self-check":
        _self_check()
        sys.exit(0)
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(1)
    result = select(load_meta(Path(sys.argv[1])))
    print(json.dumps(result, indent=2, ensure_ascii=False))
