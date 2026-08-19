# King-relative NNUE (B-small) — Phase 3 diagnostic and frozen state

Status: **read-only diagnostic, no new inference.** No training, no builds,
no engine runs, no matches were performed to produce this document — every
number below is either a field already present in an existing
`.epoch20.meta.json` sidecar (written during Phase 2's 3-seed×2-arch
training sweep) or a hash/lookup of an already-existing file/commit. Written
in response to Phase 3's mechanical `PASS` verdict looking mixed once WDL
loss and calibration error are read alongside `valid_cp_mse` (see
`scripts/select_king_relative_checkpoint.py`, which gates on `valid_cp_mse`
only and reports WDL/calibration as non-gating diagnostic context).

## 1. Frozen state (manifest)

**Phase 3 verdict: `MECHANICAL_PASS` / `EXPERIMENTAL_HOLD`** — the
pre-registered `valid_cp_mse` rule in `select_king_relative_checkpoint.py`
returned `PASS` (3/3 seeds improved), but Phase 5 (paired Elo/SPRT) is
**explicitly not authorized** pending the decision in §4. This is not a
retroactive rewrite of the `cp_mse` gate's own PASS/FAIL logic — it is an
additional decision point *between* Phase 3 and Phase 5 that was underspecified
in the original plan.

| Item | Value |
|---|---|
| PR #41 merge commit (king-relative feature landed) | `b917feba3b7a5fcd4f18e958687e9b9c714df54e` (merged 2026-08-12T13:52:39Z) |
| Training run commit (`git_commit` field, all 6 runs) | `c1719fe70b95a973a54db31acbf2919e52329604` |
| `run_king_relative_phase2.sh` / `select_king_relative_checkpoint.py` last-touched commit | `b917feba3b7a5fcd4f18e958687e9b9c714df54e` (same as PR #41 — both scripts landed in that PR) |
| Dataset (`data/gateA_csa_subset`) hash (`dataset_hash` field, all 6 runs) | `11756567284176478750` |
| Validation split hash (`split_hash` field, all 6 runs) | `15885596499200304103` |
| `split_seed` / `shuffle_seed` (fixed across all 6 runs) | `42` / `11` |
| `init_seed` (the 3-seed axis) | `42`, `7`, `123` |
| `label_depth` / `wdl_lambda` / `validation_ratio` (fixed across all 6 runs) | `4` / `0.7` / `0.15` |
| Teacher cache file (`data/teacher_cache_depth4.jsonl`) SHA-256 | `db7ce59916e39c1fbcbb1fc929a647db12b606ce770509aafab39105b2444206` |
| **Selected seed for Phase 5 (if/when authorized): `123`** | median `valid_cp_mse` among the 3 seeds, on the arch-B side — same convention `select_longrun_checkpoint.py` already uses for its own `gate_seed` pick (closest-to-median, not best-of-3). Applied mechanically post hoc here since no dedicated seed-selection field existed in `select_king_relative_checkpoint.py`'s output; recorded here in writing so it isn't re-picked after further looking at results. |
| `arch_a_seed123.epoch20.bin` SHA-256 | `c81d1a2b4006dd0372e37543b0a1b07fee18062f697d68ea7ca87e549e2f408b` |
| `arch_b_seed123.epoch20.bin` SHA-256 | `51230b0c2b5f276f1a6106cc225f7ffc454a5727e706bce5042c8804c37ed93e` |
| `checkpoint_hash` (arch_a_seed123 / arch_b_seed123, meta.json's own internal hash field) | `743a4e862d373948` / `ce260bc82fd2d5ed` |

**Why `arch_b_seed123.best.bin` is NOT the Phase 5 candidate**: `.best.bin`
is a lowest-`valid_loss`-so-far checkpoint saved during training — for
`arch_b_seed123` specifically, that best point was **epoch 3**, not epoch
20. `select_king_relative_checkpoint.py`'s own docstring is explicit that
this comparison uses each run's **final epoch**, not a best-epoch
cherry-pick within a run's own history (unlike `select_longrun_checkpoint.py`,
which does allow that, for a different comparison shape). Using `.best.bin`
here would silently swap in a different, earlier-epoch network than the one
Phase 3 actually scored — inconsistent with what was just gated.

## 2. What is and isn't available for deeper diagnostics

**No per-sample prediction file exists anywhere under
`data/runs/king_relative_phase2/`** — confirmed by directory listing (270
files, all `.bin` checkpoints, `.meta.json` aggregate sidecars, `.log`
console logs, and `.manifest.json`/`.done` bookkeeping; nothing matching
`*predict*`/`*output*`/`*sample*`). `run_king_relative_phase2.sh`'s training
invocations did not pass any tracing/dumping flag (`trainer.rs` has a
`shadow_trace`/`sample_grad_trace` mechanism for specific probe positions,
exercised only in its own unit tests — not invoked by this run). Per
instruction, **no re-evaluation job was launched** to produce one.

This means the following, requested in the original diagnostic list,
**cannot be computed** without new inference on the existing checkpoints
(out of scope for this pass):

- Prediction-value percentiles (p5/p95/median) — only mean/std/min/max/range
  are saved per epoch, which do not determine percentiles without an
  assumed distribution shape.
- Brier score, per-sample WDL log loss, reliability bin table — all
  require paired (predicted, actual) values per sample; only bucketed sums
  going into the single scalar `valid_calibration_error` were ever computed,
  and the buckets themselves were never persisted to the `.meta.json`
  sidecar (only the final reduced scalar was).
- CP-error/WDL-error correlation — requires per-sample paired values; not
  available.
- Empirical clamp-hit fraction for the current ECE calculation — no
  per-sample data to count against. An **analytic approximation** is given
  in §3 instead (explicitly not an empirical measurement).

**What *is* available** (already-saved scalars, `.epoch20.meta.json`, all
6 runs): `valid_cp_mse`, `valid_wdl_loss`, `valid_calibration_error`,
`valid_output_mean/std/min/max/range`, `pred_eval_correlation`,
`l2_dead_neurons`, and the rest of the training/architecture diagnostics
already surfaced in Phase 3's PASS verdict.

## 3. Diagnostic table (from existing aggregate data only)

| seed | arch | cp_mse | wdl_loss | calib_error | output_mean | output_std | output_min | output_max | pred_eval_corr |
|---|---|---|---|---|---|---|---|---|---|
| 42 | A | 161330.2 | 349939.1 | 0.0241 | 30.06 | 50.11 | −33.33 | 100.19 | 0.564 |
| 42 | B | 159168.0 | 350243.1 | 0.0385 | 47.89 | 89.87 | −71.37 | 159.36 | 0.659 |
| 7 | A | 162971.7 | 349040.4 | 0.0165 | 20.81 | 63.63 | −69.88 | 87.36 | 0.556 |
| 7 | B | 162401.8 | 351348.4 | 0.0459 | 56.42 | 77.66 | −37.84 | 164.86 | 0.636 |
| 123 | A | 163774.0 | 350890.3 | 0.0261 | 32.15 | 60.19 | −44.87 | 100.47 | 0.540 |
| 123 | B | 161955.1 | 355815.6 | 0.0553 | 57.64 | 85.48 | −40.82 | 173.77 | 0.639 |

**Analytic clamp-fraction estimate** (assumes each arch/seed's output is
approximately normal with the observed mean/std; `predicted_prob` clamps
when `|score| >= 0.5 * wdl_target_scale = 600`): **≈0.0% for every one of
the 6 runs.** The largest observed `|score|` anywhere in the data is
173.77 (B, seed 123) — nowhere near the ±600 clamp boundary. **This
refutes the clamp-confound hypothesis raised when Phase 3's mixed result
was first reported** (that B-small's ~1.8× wider `output_std` was
mechanically inflating measured ECE via boundary clamping). A sigmoid or
logistic remapping would not materially change this picture either, by the
same reasoning — none of the observed scores are large enough in magnitude
for a hard linear clamp to be biting in the first place, and a smooth
sigmoid saturates even later than the linear map's clamp boundary.

**What the data does support, from already-available scalars only**:
`pred_eval_correlation` is higher for B-small in every seed (0.64–0.66 vs.
A's 0.54–0.56) — its raw score is *more* informative about the target than
A's, not less. At the same time, B-small's `output_mean`/`output_std` are
both substantially larger than A's in every seed (roughly 1.4–1.8× on
mean, 1.3–1.8× on std). A plausible reading, **not verified without
per-sample data**: B-small's outputs occupy a different, "hotter" scale
than A's, and `valid_wdl_loss`/`valid_calibration_error`'s fixed linear
`score/1200 + 0.5` mapping (implicitly tuned around A's typical scale
during this project's history) penalizes that scale mismatch even where
the underlying ranking/signal quality (per `pred_eval_correlation`) is
better, not worse. This is a genuinely different hypothesis than the
(now-refuted) clamping one, and — like the clamping hypothesis — it is
**not confirmed**, only consistent with the aggregate numbers on hand.
Confirming it would require either per-sample data (new inference, out of
scope here) or a rescaling/recalibration experiment (new training or
post-hoc calibration fitting, also out of scope here).

## 4. Pre-registered decision rule (written before any further analysis)

**Conditions to proceed to Phase 5's smoke test:**
- `valid_cp_mse` 3/3 improvement is maintained (already true, §3).
- The bulk of the ECE/WDL degradation is confirmed to be
  mapping/scale-dependent rather than a genuine ranking-quality regression
  — **not yet established**; §3's clamping-specific version of this is
  refuted, but the broader scale-mismatch hypothesis remains unconfirmed.
- Brier score or a properly-calibrated WDL log loss is not catastrophically
  worse — **not computable from existing data** (§2).
- No collapse or extreme tail in the output distribution — checked:
  `valid_output_std` never drops below `COLLAPSE_STD=5.0` in any run (all
  ≥50.1), so no collapse by the existing gate's own threshold; "extreme
  tail" beyond min/max range is not otherwise measurable without per-sample
  data.

**Conditions to hold B-small here:**
- Even after a corrected mapping, WDL/calibration clearly worsens 3/3 —
  **not yet determined**, since no corrected-mapping recomputation was
  performed (would require per-sample data or new inference).
- The CP improvement is small relative to the probability-quality
  degradation — on raw numbers, CP gains are small (0.35–1.34%) against
  parameter count growth (8.3×) and calibration error roughly doubling to
  tripling in every seed; this reads as **already leaning toward "small
  gain, real cost"** even before a corrected-mapping recomputation, though
  not decisively so given §3's open scale-mismatch question.
- Direction of improvement is unstable across seeds — not the case for
  `cp_mse` (3/3 same direction); also not the case for
  `wdl_loss`/`calibration_error` (3/3 same direction, consistently worse).
  Both metrics are internally *consistent* across seeds; they simply
  disagree with each other on which architecture is better.

This document does not resolve which side of that disagreement should
win — that determination requires either new data (per-sample predictions,
a corrected calibration mapping, or a rescaling experiment) or a judgment
call this document deliberately leaves to the reader.

## 5. Summary for the frozen record

- `valid_cp_mse`: 3/3 seeds improved (B-small better).
- `valid_wdl_loss`: 0/3 seeds improved (B-small worse in every seed).
- `valid_calibration_error` (current linear-clamp ECE): 0/3 seeds improved
  (B-small worse in every seed, by roughly 1.6×–2.1×).
- B-small has 8.3× the parameter count of A (5.28M vs. 636K).
- Measured NPS with real trained weights, `Threads=2`, `go depth 12`,
  same seed-42 checkpoints: A = 504,650 nps, B = 452,211 nps (≈10% lower,
  not alarming on its own).
- The clamp-boundary explanation for the calibration/WDL gap is refuted by
  the data (§3); a scale-mismatch hypothesis remains open and unconfirmed.
- No Elo/strength claim exists for B-small at any point in this
  investigation.
- **Phase 5 (paired Elo/SPRT) remains explicitly on hold**, pending a
  decision on §4's still-open conditions.
