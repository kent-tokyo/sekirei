# Official NNUE v1, Tier 2 — 3-seed training report

Reports execution of the preregistered Tier 2 recipe
(`docs/experiments/official_nnue_v1_preregistration.md`, PR #55) on the real, full CSA corpus. This document
covers **training execution and candidate selection only** — it is not Gate 1 (that is its own branch,
`docs/official-nnue-v1-validation`, per the preregistration's "Gate 1" section) and stamps no strength or
quality verdict.

- **Phase**: Tier 2, Phase 2 (3-seed production training). Phase 1 (teacher-cache warm-up) and dataset sizing
  (PR #60) preceded this and are not re-described in full here.
- **Base SHA**: `c9b95ad9fe5c498902f0d3a806e97e540e070d86` (main tip after PR #60 merged; recorded directly from
  each checkpoint's own `meta.json` `git_commit` field, not asserted from memory).
- **Branch / worktree**: `experiment/official-nnue-v1-tier2-training`,
  `/Users/k_tanabe/Documents/Documents/oss_rust/sekirei-nnue-v1-tier2-train`.
- **Changed files (this PR)**: this document only. Trained artifacts (`data/runs/nnue_v1_tier2/*`) live under
  `data/`, which is gitignored project-wide — they are not committed; SHA-256 checksums below make them citable
  and the run is independently reproducible (determinism verified, see below).

## Commands run

Phase 1 (teacher-cache warm-up, completed prior to this phase; recorded for completeness): three
`--games` runs over 100-file chunks (`/tmp/csa_chunk1`/`2`/`3`, `--init-seed 1`, `--epochs 1`, discarded output
weights), all sharing `--teacher-cache data/runs/nnue_v1_tier2/teacher_cache.bin --reuse-teacher-cache`.
Final cache: **5,159 entries**.

Phase 2 (this report), exactly the preregistered recipe, `--init-seed` varying, everything else fixed:

```sh
cargo run --release -q -p sekirei-train -- \
  --games /tmp/csa_full300 \
  --label-depth 4 --wdl-lambda 0.7 \
  --min-ply 20 --min-rate 1800 \
  --validation-ratio 0.15 --split-seed 42 --shuffle-seed 7 --init-seed {7|42|123} \
  --epochs 20 \
  --teacher-cache data/runs/nnue_v1_tier2/teacher_cache.bin --reuse-teacher-cache \
  --checkpoint-dir data/runs/nnue_v1_tier2/checkpoints_seed{7|42|123} \
  --output data/runs/nnue_v1_tier2/candidate_seed{7|42|123}.bin
```

`/tmp/csa_full300` is a symlink directory unioning all 300 files from `data/csa/2023/` (the same files split into
the three Phase 1 warm-up chunks). Run three times, once per `--init-seed`.

**Not run**: Gate 1 (training-diagnostic verdict), Gate 2 (analysis-quality comparison), Gate 3 (SPRT strength
gate), any match/benchmark generation. No merge, version bump, tag, release, or publish performed.

**Determinism re-verification**: re-ran the seed-7 command a second time (scratch output paths, full untruncated
log). Both the final-epoch weights and the best-checkpoint weights were byte-identical (`cmp`) to the first run's
artifacts — reconfirms this project's established determinism property on the `--games` path specifically.

## Resource state

Checked immediately before Phase 2 (per this project's standing 5-signal resume checklist) and again mid-run:

| Signal | Before Phase 2 | 30s later |
|---|---|---|
| Load average (1/5/15m) | 3.04 / 2.03 / 1.81 | 2.69 / 2.10 / 1.85 (10-core machine) |
| Swap used | 8994.06M / 10240M | 8986.06M–8994.06M (flat, not climbing) |
| Free RAM (pages) | 3,868 pages (~60MB) | — |
| Competing heavy processes | none (prior day's `mds_stores`/`mds` Spotlight-reindex load, ~9.5 load avg, had cleared) | — |
| Leftover Sekirei processes | none | — |

Disk: 12Gi used / 17–18Gi free (40–41%) throughout, essentially unchanged (checkpoints are small; see below).

**Actual per-seed cost, cache-warm**: 43.9–45.4s wall-clock each (seed7: 45.39s, seed42: 43.93s, seed123: 44.04s),
~97–99% single-core CPU utilization (user time ≈ wall time), **0 cache misses in any epoch of any seed**
(5,182 cache hits/epoch × 20 epochs × 3 seeds). This is the payoff of Phase 1's warm-up: the ~30-minute
background-kill risk that motivated chunking Phase 1 does not apply to Phase 2 at all — each seed run is two
orders of magnitude under that window.

## Artifact SHA-256

```
e4da09316ef8e5892ea58f1a338b13851ff9db54b11b5634aac2492fd05d8da4  candidate_seed7.best.bin
ce272edbcc53312982b447bfcc8e6a5476dd770937687bdac9f0627b77b3f314  candidate_seed42.best.bin
1c287b0ce6b31cf6b9f71b581c5789642073b6c0a0e6b69178643585a589fc8c  candidate_seed123.best.bin
36b7887d38b1d6e8008489d22507a356bd86005b41cc6ad7ad089f7eb405c28a  teacher_cache.bin (5,159 entries)
```

(`.bin` = final/epoch-20 weights; `.best.bin` = the trainer's own best-valid_loss-tracked checkpoint per run,
per the preregistration's "checkpoint selection" rule — epoch 3 for all three seeds, see below.)

## Dataset / split / teacher identity

`dataset_hash`, `split_hash`, and `git_commit` in every seed's `meta.json` are identical across all three runs
(`dataset_hash=8080631893926094213`, `split_hash=2827055254973556117`) — confirms `--init-seed` was the only
varying factor, as preregistered.

- Games dir: `/tmp/csa_full300` (300 raw CSA files, symlinked from `data/csa/2023/`).
- **`176` games survived the `--min-rate 1800` filter** → `154` train / `22` valid (`--validation-ratio 0.15`,
  `--split-seed 42`).
- **`5,182` total labeled positions** → `4,486` train / `696` valid.
- Teacher cache: 5,159 entries, all consumed as hits (0 misses) — `label-depth 4`, `wdl-lambda 0.7`.

**Deviation from PR #60's sizing extrapolation, flagged not hidden**: PR #60 projected ~246 post-filter games /
~7,700 positions for a 300-file corpus (linear extrapolation from its own 100-file measurement, itself already
labeled "not independently verified at that scale"). Actual: 176 games (−28.5%) / 5,182 positions (−32.7%). Not
investigated further here — plausibly a non-uniform game-count or rating-pass-rate distribution across the
300-file range vs. the 100-file sample PR #60 used, but that is a guess, not a finding; flagged as open, not
resolved.

## Training dynamics — a real finding, not a footnote

The preregistration deliberately left `--lr-schedule` at the trainer's default rather than specifying cosine
(reasoned in PR #55). The default is `StepHalf`: LR halves every epoch from `0.001`, underflowing to
`0.000000` (printed) by epoch 15. `update_norm` (the per-epoch parameter-change norm) collapses in lockstep:
`35.4` (epoch 2) → `12.5` (epoch 3) → `5.3` (epoch 4) → `0.005` (epoch 14) → `~0.00003` (epoch 20), for seed 7;
the same pattern holds for seeds 42 and 123.

**Consequence**: `--epochs 20` was preregistered as "an upper bound... not a claim epoch 20 is the right
stopping point," and that turned out true in a stronger sense than anticipated — **all three seeds picked
epoch 3 as their best-valid_loss checkpoint**, and every later epoch is effectively frozen (near-zero further
update). The real, effective training budget this run exercised was closer to ~4 epochs over 4,486 positions
than the nominal 20-epoch/5,182-position recipe. This directly conditions how Gate 1 should interpret these
diagnostics — it measures a schedule-truncated run, not a fully-exercised 20-epoch one. Not fixed or rerun here
(no epochs/schedule change is in this report's scope); flagged for the next stage's interpretation and as
candidate discovered work (below).

## Metrics — all three seeds, best (epoch 3) checkpoint

| seed | valid_loss (best, epoch3) | valid_loss (epoch20/final) | cp_mse | wdl_loss | calibration_error | l2_dead_neurons /32 | l2_ever_saturated |
|---|---|---|---|---|---|---|---|
| 7   | **178383.7306** | 178824.0835 | 205314.7739 | 338471.4593 | 0.1867 | 8  | 0.750 |
| 42  | 180858.9685     | 181361.1104 | 204873.5283 | 347751.8267 | 0.2019 | 14 | 0.5625 |
| 123 | **178121.1924** | 179831.2976 | 204022.4028 | 340611.8647 | 0.1942 | 9  | 0.6875 |

`l2_dead_neurons`/`l2_ever_saturated` are Gate 1 inputs (its "no saturation/dead-neuron collapse" check), not
graded here. Per `tasks/lessons.md`'s 2026-07-14 entry, `l2_ever_saturated==l2_ever_active` is a naming
artifact ("touched the ceiling at least once during the run"), not "stuck at ceiling" — same caveat applies to
these numbers.

No zero-init-style uniform-row collapse observed in any of the three (`ft_active=1.000`, non-uniform
`l2_row_weight_norm_per_neuron` values per checkpoint's `meta.json` — spot-checked for seed 7, not tabulated
here for space).

## Median-seed selection — an unresolved ambiguity, flagged rather than silently decided

The preregistration states: *"Seed selection for the candidate that proceeds to Gate 1: median, not best, of
the 3 runs' **final** `valid_loss`."* This sentence is ambiguous between two readings, and **they select
different seeds**:

- **Reading A — "final" = each run's selected (best-tracked) checkpoint's valid_loss** (consistent with the
  immediately preceding bullet, "checkpoint selection: the trainer's own built-in best-valid_loss tracking"):
  sorted `[123: 178121.19, 7: 178383.73, 42: 180858.97]` → **median = seed 7**.
- **Reading B — "final" = literally epoch 20's valid_loss**: sorted `[7: 178824.08, 123: 179831.30,
  42: 181361.11]` → **median = seed 123**.

Reading A is what this report recommends, for two reasons: (1) it is the self-consistent one — the artifact
that actually ships per-seed is each run's *best* checkpoint (epoch 3 for all three), so ranking seeds by a
different epoch's (epoch 20's) metric while shipping the epoch-3 file would rank seeds by a number that isn't
the one attached to the shipped file; (2) Reading B would make seed 123 the "median" seed under the ranking
metric, while seed 123's *actual shipped checkpoint* (`.best.bin`, epoch 3, valid_loss=178121.19) is the
**lowest of all three** at epoch 3 — i.e., Reading B smuggles in exactly the "best-of-3 cherry-picking" the
rule's own stated purpose (*"a 'best of 3' checkpoint is partly measuring luck, not recipe quality"*) says to
avoid.

**This report does not unilaterally resolve the ambiguity** — it is a real fork in what ships to Gate 1, not a
formatting nit. **Recommended: seed 7's `candidate_seed7.best.bin`** (Reading A). Holding for explicit
confirmation before that file is passed to Gate 1.

## Discovered work (not fixed here, out of this branch's scope)

- **`--checkpoint-dir` is a no-op on the `--games` path.** Verified directly in
  `crates/sekirei-train/src/main.rs`: the `--positions` path's epoch-checkpoint save (line ~2021) does
  `checkpoint_dir.join(...)`, honoring the flag; the `--games` path's epoch-checkpoint save (line ~2585) uses
  `args.output.with_extension(...)` instead, silently ignoring `args.checkpoint_dir` entirely. No error or
  warning is printed when `--checkpoint-dir` is passed on the `--games` path — this is why
  `data/runs/nnue_v1_tier2/checkpoints_seed{7,42,123}/` were never created; all epoch checkpoints landed next to
  `--output` instead (harmless here, since that is exactly where this report's SHA-256 checksums point, but a
  real inconsistency between the two ingestion paths worth a small follow-up fix).
- **LR-schedule-driven early effective convergence** (above) — worth considering whether a future Tier-2-scale
  run should either (a) preregister a lower `--epochs` given `StepHalf`'s decay, or (b) explicitly pin a
  schedule matched to the intended epoch budget, rather than relying on the trainer's default.

## Gate result

**Not a gate.** This report covers training execution only. Scoped verdict for *this step's own execution*:

**PASS (execution)** — all three runs executed exactly as preregistered, dataset/split identity confirmed
identical across seeds, determinism re-verified byte-for-byte, no zero-init-style collapse, 0 cache misses. This
is not a quality, strength, or Gate-1 HEALTHY/WARNING verdict.

## Next operation

**Gate 1 (training-diagnostic validation, its own branch `docs/official-nnue-v1-validation` per the
preregistration)** — compute the `HEALTHY`/`WARNING`/`INSUFFICIENT_DATA`/`INVALID` verdict from the metrics
already gathered above. **Not heavy**: no new training/labeling/compute needed, purely analysis of data already
on disk. **Not started this round** — held pending (a) the median-seed ambiguity resolution above, since Gate 1
grades a specific candidate file, and (b) this project's established per-phase stop-and-report pattern (every
prior Tier 1/Tier 2 phase this session ended with a report + PR and waited for explicit "merge and next step"
rather than self-chaining into the next phase).

## Merge status

Not merged. PR to be opened from this branch, reporting only, per this roadmap's standing "no auto-merge" rule.

## Items needing approval / explicit confirmation

1. **Median-seed reading (Reading A vs. B above)** — recommend Reading A (seed 7). Needs explicit confirmation
   before Gate 1 grades a specific file.
2. **Whether to proceed to Gate 1 next** (cheap, analysis-only, no heavy compute) once (1) is resolved.
3. **Whether the `--checkpoint-dir`/`--games` no-op (discovered work, above) is worth a small standalone fix
   PR**, and if so, whether it belongs on its own branch (recommended, since it is unrelated to Tier 2's own
   scope) rather than this one.
