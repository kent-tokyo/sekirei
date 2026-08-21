# Official NNUE v1 — Tier 1 (P0 pipeline pilot) report

Base SHA: `7a5d79c` (`docs/experiments/official_nnue_v1_preregistration.md`'s merge commit). Branch:
`experiment/official-nnue-v1-pilot`. **Not an official candidate** — per the preregistration, this run's own
metrics feed into nothing downstream (no Gate 1-4 decision reads this document). Purpose was pipeline
correctness + real resource measurement, both achieved.

## Command run (exactly as preregistered, no changes)

```sh
cargo run --release -q -p sekirei-train -- \
  --positions data/runs/bc_redo_20260628_214103/stage1/positions_10k.jsonl \
  --scored data/runs/bc_redo_20260628_214103/stage3/scored_10k.jsonl \
  --stability-weighted --min-stability 0 \
  --label-depth 4 \
  --validation-ratio 0.1 --split-seed 42 \
  --init-seed 42 \
  --epochs 3 \
  --checkpoint-dir data/runs/nnue_v1_pilot/checkpoints \
  --output data/runs/nnue_v1_pilot/weights_pilot.bin
```

`data/` is a symlink to the main checkout's `data/` in this worktree (untracked, not shared automatically across
`git worktree`s; avoided a 7.3 GB copy on an already-96%-full disk).

## Result: PASS on pipeline correctness, but the resource estimate was wrong — reported honestly, not revised quietly

- Exit code 0. Produced `weights_pilot.bin`, `weights_pilot.best.bin`, 3 per-epoch checkpoints + `.meta.json`
  each, all well-formed (`arch_name: "A-flat-ps"`, `architecture: "INPUT=2420 L1=256 L2=32"` — matches every
  other checkpoint in this project).
- `valid_loss` (raw, position-level-split, cp-only — **not the model-card's `valid_cp_mse`**, different metric
  name and scale, not directly comparable without conversion this report doesn't attempt) improved monotonically
  across all 3 epochs: 222983.62 → 217356.60 → 214645.64. No NaN, no explosion.
- No collapse signature: `ft_active=1.000` held through all 3 epochs (no dead-FT collapse); `l2_dead` grew 3→13→13
  (out of 32 L2 units) — some dead L2 neurons, stable, not all-32 collapse. `pred_eval_corr` rose 0.249 → 0.427 →
  0.467 across epochs.
- The trainer's own built-in best-checkpoint selection worked exactly as preregistered: `best (valid_loss=214645.6364)
  → weights_pilot.best.bin (from weights_pilot.epoch3.bin)` — mechanical, not a judgment call.
- `wdl_component=n/a` in every epoch — confirms the preregistered, expected limitation (the `--positions` path
  has no `game_result`, so no WDL signal here; this was never claimed as a Gate-1-ready diagnostic).
- `missing_rate=2.9%` (260/9708 scored samples dropped as unmatched against the teacher cache) — not
  investigated further this round, noted for whoever runs Tier 2.

**The wall-clock estimate in the preregistration was wrong by roughly two orders of magnitude.** Preregistration
said: "a rough order-of-magnitude guess is low-single-digit minutes... explicitly a guess." Actual: **2 hours 29
minutes 14.75 seconds wall-clock** (`8971.71s user + 135.31s system`, **101% CPU** — effectively single-threaded,
not using this machine's other 9 cores). This is reported as a correction, not smoothed over — the guess was
wrong, and Tier 2's own resource estimate (3 seeds × 20 epochs, on a dataset not yet sized) cannot be responsibly
projected from "3 epochs on 9,708 positions took 2.5 hours" without first separating one-time cost from
per-epoch cost, which this run's own log doesn't isolate (only one combined total is printed; epoch-level
wall-clock wasn't captured). **Recommendation, not yet acted on**: instrument per-epoch wall-clock (or rerun
with fewer epochs, e.g. `--epochs 1`, to isolate the label-generation-dominated first epoch from a
labeling-free second epoch) before committing to any Tier 2 time budget.

## Disk

| | Free (`/System/Volumes/Data`) |
|---|---|
| Before | 5.2 GiB |
| After | 4.2 GiB |
| Pilot's own output size | 6.3 MB (`du -sh data/runs/nnue_v1_pilot`) |

The ~1 GiB drop over the run's 2.5 hours is **not from this pilot** (its own footprint is 6.3 MB, far under the
500 MB budget) — consistent with the unrelated background disk churn already noted elsewhere this session. Within
the preregistered 2 GiB abort tripwire; no abort was needed. Absolute free disk (4.2 GiB) is now lower than at
preregistration time (9.3 GiB) purely from unrelated system activity — worth a fresh check, not an assumption,
before Tier 2.

## Artifact identity

- `weights_pilot.best.bin` SHA-256: `2d02c659c55eef4bfec9938fb7e141db2c9c2a3add52fb5140a16f69a8fe9c3f`
- `positions_10k.jsonl` SHA-256: `c43796f06dbe0c89c247ae992a6f081482f1f837357db8266da3a524767be935`
- `scored_10k.jsonl` SHA-256: `74b935e721aba44f1d131ecd5c59740935f73639e641b0eb2616d27f3dd7eec0`
- `checkpoint_hash` (trainer-internal, from `.meta.json`): `d5a1f8641269dc32`

Checkpoint `.bin` files are **not committed** (matches every prior checkpoint in this project's history; `data/`
stays gitignored).

## What this does and does not license

Does: confirm the `--positions`/`--scored` pipeline still runs end-to-end on this exact repo checkout, produces
a well-formed, non-degenerate checkpoint, and gives one real (if not yet fully decomposed) wall-clock data point.

Does not: say anything about Tier 2's likely outcome (different code path, different split semantics, WDL
present, 3 seeds, larger dataset — see the preregistration's own "Tier 1's results say nothing directly about
Tier 2's outcome"). Does not change any PASS/HOLD/FAIL status — none was pending on this run.

## Next step

Not started without separate approval: Tier 2 (Official NNUE v1 recipe), the dataset-sizing measurement plan, or
any further training. Recommend resolving the per-epoch-vs-one-time-cost question above before setting a Tier 2
time budget.
