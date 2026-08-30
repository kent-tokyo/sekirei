# WDL target-scale matched ablation (`--wdl-target-scale`): shrinks the gradient ratio proportionally, doesn't reach parity, and worsens epoch-3 dead-neuron count

## Background

`docs/experiments/cp_wdl_target_residual_trace.md` found that `wdl_target` (a fixed `±600` constant per game,
`(wdl - 0.5) * 1200.0`) structurally dominates the blended gradient over `eval_teacher`'s fine-grained per-position
signal — not primarily because of `--wdl-lambda`'s weighting, but because of a target-scale/structure mismatch.
Per that doc's own calibration: this didn't yet justify "the WDL target is bad," only that mixing it at the same
numeric range as CP is unstable. This experiment tests the direct, targeted fix implied by that framing: does
shrinking `wdl_target`'s native range change the epoch-1 collapse or the gradient imbalance?

**Implementation**: `wdl_target_cp` took a new `scale: f32` parameter (`Some((wdl - 0.5) * scale)`), threaded as a
per-call parameter through `position_teacher_components`/`train_game`/`eval_game`, mirroring `--wdl-lambda`'s
existing parameter-based threading. New CLI flag `--wdl-target-scale <f>`, default `1200.0` (reproduces today's
`±600` exactly when omitted). Verified before running the ablation: `cargo test`/`fmt`/`clippy` clean (90 tests,
including the 6 existing `wdl_target_cp` unit tests updated to pass `1200.0` explicitly); a live run with the flag
omitted produces a byte-identical `.bin` and identical `.meta.json` to the same run with `--wdl-target-scale 1200.0`
passed explicitly; an `--epochs 1` scale sweep confirmed `wdl_component` (the WDL squared-error loss term) shrinks
roughly with `scale²` as expected (`352307 → 39381 → 9719` for `1200/400/200`, close to the `9×`/`36×` predicted by
`(1200/400)²=9`, `(1200/200)²=36`). `wdl_target_scale` is also written into every checkpoint's `.meta.json`, so the
actual scale used is recoverable from any run's metadata alone, not just its invocation log.

## Design

Single variable: `--wdl-target-scale` — control `1200.0` (`±600`, today's behavior), candidate 1 `400.0` (`±200`),
candidate 2 `200.0` (`±100`). Fixed everywhere else: `--wdl-lambda 0.7`, `data/gateA_csa_subset`, `--split-seed 42`,
`--lr-schedule cosine --min-lr 0.00001 --warmup-epochs 1`, `--epochs 3`, 3 seeds (`--init-seed` 42/7/123), one
shared teacher-search cache (already fully populated from a prior experiment's runs on the same games/split-seed/
label-depth — every run in this ablation hit `cache_miss=0`). `--cp-wdl-grad-trace --trace-positions
0,1,2,4,8,16,32,64,128,256` enabled on every run. 9 runs total (3 scales × 3 seeds).

## Pre-registered decision framework (fixed before running)

Promote only if all four hold:
1. CP/WDL gradient ratio is no longer extreme (not just smaller — the candidate's ratio at positions 1-16 should
   not still be orders-of-magnitude skewed the way control's `~18,500×-308,000×` was).
2. Epoch-1 collapse (dead-neuron count, `valid_output_std`) measurably improves vs. control.
3. `valid_cp_mse` does not regress vs. control, any epoch.
4. All 3 seeds agree in direction.

Any single condition failing → not promoted.

## Result 1: the gradient ratio shrinks exactly linearly with scale — but never reaches parity

L2-layer `wdl/cp` gradient-RMS ratio, mean across 3 seeds, epoch 1:

| position | ctrl600 (±600) | cand200 (±200) | cand100 (±100) |
|---|---|---|---|
| 1 | 126,012 | 42,005 | 21,003 |
| 2 | 34,600 | 11,533 | 5,767 |
| 4 | 19,164 | 6,388 | 3,194 |
| 8 | 10,432 | 3,477 | 1,738 |
| 16 | 5,124 | 1,707 | 853 |
| 256 | 1.87 | 0.58 | 0.28 |

The ratio at every position scales almost exactly linearly with `wdl_target_scale`: `1200→400` is a 3× scale
reduction and drops the position-16 ratio 3.0× (`5124→1707`); `1200→200` is a 6× reduction and drops it 6.0×
(`5124→853`). This is the clean, mechanical core finding — `wdl_target`'s own magnitude sets the WDL residual's
magnitude close to linearly, exactly as the squared-error-gradient math predicts.

**It still doesn't resolve condition 1.** Even at `±100` — 6× smaller than today's default — positions 1-16 are
still `853×` to `21,003×` WDL-dominant, the same orders-of-magnitude skew the framework's bar was written against.
Driving the early-position ratio down to something no longer "extreme" would require `scale` near 0, i.e. near-
elimination of the WDL signal, not a moderate rescaling. (Position 1's absolute values are noisier across seeds —
`cp_target=0` there in the unshuffled window, so `cp_gradient` is a near-zero, seed-dependent artifact rather than
a stable signal; positions 2-16 are the more trustworthy read and show the same clean linear pattern.)

**Position 256 overcorrects in the other direction.** Control is WDL-dominant there (`1.87×`); `cand200` and
`cand100` both flip to CP-dominant (`0.58×`, `0.28×`). No single fixed scale balances the ratio across the position
range that matters — early positions need WDL pulled down far more aggressively than late positions do, which a
constant scale factor structurally can't provide.

**Condition 1: fails**, for both candidates.

## Result 2: epoch-1 metrics — mixed, no clean win

Per-epoch metrics, mean across 3 seeds (raw per-seed dead-neuron counts in parentheses):

| scale | epoch | `valid_cp_mse` | `valid_output_std` | `l2_dead_neurons` | `l2_sat_freq` |
|---|---|---|---|---|---|
| ctrl600 | 1 | 173,319 ± 355 | 6.47 ± 11.20 | 3.33 ± 1.53 (2,5,3) | 0.341 |
| cand200 | 1 | 173,037 ± 895 | 2.39 ± 4.15 | **1.00 ± 1.00 (2,1,0)** | 0.354 |
| cand100 | 1 | 173,025 ± 989 | 4.50 ± 7.79 | 2.67 ± 2.52 (0,5,3) | 0.300 |

At epoch 1 alone, `cand200`'s mean dead-neuron count looks better (1.00 vs. control's 3.33) and `cand100`'s is
roughly flat. `valid_output_std` is noisy in both directions and not a clean win either way. **Condition 2 is at
best a weak, single-candidate, not-clearly-seed-consistent signal** — reading only this snapshot would already be
an incomplete picture (see Result 3 below).

## Result 3: epoch 2-3 — the aggressive candidates get *worse*, not better

| scale | epoch | `valid_cp_mse` | `valid_output_std` | `l2_dead_neurons` | `l2_sat_freq` |
|---|---|---|---|---|---|
| ctrl600 | 2 | 168,723 ± 6,332 | 27.32 ± 27.05 | 5.00 ± 4.36 (8,7,0) | 0.312 |
| ctrl600 | 3 | 168,578 ± 6,166 | 28.92 ± 26.79 | 5.33 ± 2.89 (7,7,2) | 0.424 |
| cand200 | 2 | 163,654 ± 433 | 40.23 ± 6.23 | 3.67 ± 6.35 (0,0,11) | 0.187 |
| cand200 | 3 | 163,424 ± 360 | 40.31 ± 5.59 | **12.67 ± 2.89 (11,11,16)** | 0.161 |
| cand100 | 2 | 167,271 ± 4,984 | 32.95 ± 21.98 | 7.33 ± 4.04 (11,8,3) | 0.239 |
| cand100 | 3 | 166,740 ± 5,320 | 33.22 ± 21.73 | **11.33 ± 7.64 (13,18,3)** | 0.195 |

By epoch 3, `cand200`'s dead-neuron count (`12.67`, raw `11,11,16`) is worse than control's (`5.33`, raw `7,7,2`)
**in all 3 seeds** — a directionally consistent regression, not noise. `cand100` shows the same pattern (`11.33`
vs. `5.33`, raw `13,18,3`). This is the reverse of what epoch 1 alone suggested for `cand200`: the epoch-1 snapshot
looked like a mild improvement, but the full 3-epoch trajectory shows the more aggressively-rescaled candidates
accumulating *more* dead neurons than control by the end of training, not fewer. This is pre-registered metric 4
(`l2_dead`, all 3 epochs) — it's in-framework, not an incidental discovery, and it's the headline result of this
ablation, not a footnote to condition 1's failure.

`valid_cp_mse` does not regress under either candidate at any epoch — both are flat-to-better than control
throughout. This isn't read as a genuine benefit: pulling less WDL signal into the blended target moves the
network's output closer to a CP-only fit almost tautologically, so a same-or-better `valid_cp_mse` is expected and
is not, on its own, evidence the scale change is good. **Condition 3 passes as a guard, not a win.**

**No mechanism is claimed for the epoch-3 dead-neuron regression.** It's reported as observed and consistent across
seeds; explaining *why* smaller `wdl_target_scale` correlates with more dead neurons by epoch 3 is left open.

## Applying the pre-registered decision framework

1. **Fails.** Ratio shrinks linearly with scale but never approaches non-extreme at either candidate value.
2. **Weak/mixed**, and reverses by epoch 3 (see below) — not a clean pass even considered alone.
3. **Passes**, but as an expected guard rather than a signal in favor of the change.
4. **Mixed.** The ratio-shrinkage direction is seed-consistent; the epoch-3 dead-neuron regression is *also*
   seed-consistent, but in the adverse direction.

**Not promoted.** Condition 1 fails cleanly on its own, which is sufficient to close this out regardless of the
other three. This is not a plain bucket-4 ("no improvement") result like the prior three closed experiments in
this line (clipping, warmup, l2-bias-init) — the more aggressive candidate (`±100`) is **mildly harmful by epoch
3**, via a metric the framework was already tracking.

## Conclusion

**Target-scale rescaling alone is the wrong lever.** The gradient-ratio imbalance responds linearly and
predictably to `wdl_target_scale` — confirming the mechanism identified in `cp_wdl_target_residual_trace.md` is
real and controllable — but a *constant* scale factor cannot fix a ratio problem that is worst at the earliest
positions and mild by position 256: shrinking the constant enough to fix early positions overcorrects late
positions, and shrinking it only partway (as tested here) leaves early positions still orders-of-magnitude
skewed while introducing a new, seed-consistent epoch-3 dead-neuron regression.

**Not pursuing a further scalar/schedule-style lever.** A WDL-introduction schedule (ramping the effective WDL
weight in from near-zero rather than a fixed `--wdl-lambda`) was flagged as a deferred idea in the prior doc, but
this experiment is itself the fourth single-scalar/magnitude-style lever tried against the epoch-1 collapse line
(grad clipping, LR warmup, `--l2-bias-init`, now `--wdl-target-scale`) — all four have failed to touch the
mechanism, or in this case actively worsened it. A schedule is the same family of fix (still a global weighting
knob) and doesn't address *why* WDL loss pushes L2 into the dead zone in the first place. **Next direction instead**:
trace the actual backprop pathway from the output layer through L2 that WDL's loss term drives — i.e. inspect the
output→L2 backward structure directly, or look at sample-level (not just aggregate) gradient correlation, to find
the mechanism rather than another global-weighting scalar. Not started; no implementation exists yet.

## Status

- `--wdl-target-scale` stays in the codebase as an optional flag, default `1200.0` (unchanged behavior unless
  passed) — the mechanism is real and may be a useful research knob even though this sweep didn't find a promotable
  fixed value.
- **Not promoted.** Neither `400.0` nor `200.0` is adopted as a new default.
- Full per-run data (9 runs, 3 epochs each, `.meta.json` + `.trace.json` per epoch): `wdl_scale_exp/` in scratch.
