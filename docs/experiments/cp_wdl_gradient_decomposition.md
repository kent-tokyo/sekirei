# CP/WDL gradient decomposition: WDL dominates the epoch-1 push, magnitude more robustly than direction

## Background

The `--trace-positions`/`--shuffle-seed` experiment (`docs/experiments/epoch1_batch_trace.md`) established that data
order has a material, directly-observed effect on the epoch-1 L2 collapse, and resolved which neurons to look at.
This experiment decomposes the blended training gradient — `teacher = λ·eval_teacher + (1-λ)·wdl_target`,
λ=0.7 — into its CP-only and WDL-only contributions, to answer which term is actually pushing neurons toward the
dead zone or toward revival, per the user's explicit next-priority instruction.

**Implementation** (`--cp-wdl-grad-trace`, separate commit from the trace-tool work): the blended loss is
mathematically the sum of two independent squared-error terms with additive gradients (documented in
`trainer.rs`'s own module comment), so decomposition doesn't require new closed-form derivation — `diagnostic_backward`
recomputes the backward pass `train_position` already ran, twice more, once with `err = score - eval_teacher` and
once with `err = score - wdl_target`, structurally mirroring the tested main pass line-for-line (deliberately not
refactored to share code, so it's trivially auditable rather than trusting a new derivation). Never touches
`self.weights`, `self.weights.step`, or any Adam moment. Off by default; two extra diagnostic-only backward passes
per position when on.

**Verified before trusting any output** (both required, both pass):
- `cp_wdl_grad_trace_blended_gradient_is_the_expected_weighted_sum`: the real blended-gradient accumulator
  (`l2_dacc_sum`/`ft_dacc_sum`, already used by `--trace-positions`, untouched by this flag) equals
  `λ·cp_dacc_sum + (1-λ)·wdl_dacc_sum` at every neuron, within float tolerance.
- `cp_wdl_grad_trace_does_not_alter_training_state`: identical seed/inputs, flag on vs. off, produces byte-identical
  trained weights, `total_loss`, and every other accumulator — confirmed again live (not just in the unit test): the
  real epoch-end `l2_dead` count for seeds 7/42/123 with the flag *on* matched the pre-existing baseline exactly.

## Result: WDL dominates in magnitude, robustly; WDL's near-perfect directional consistency does not replicate under shuffling

**Whole-layer gradient RMS, seed 7, unshuffled** (`data/gateA_csa_subset`, same recipe as every experiment in this
line):

| position | L2 `cp_grad_rms` | L2 `wdl_grad_rms` | ratio (wdl/cp) |
|---|---|---|---|
| 1 | 0.014 | 250.961 | 18,515× |
| 8 | 0.009 | 157.110 | 16,588× |
| 16 | 0.017 | 140.960 | 8,284× |
| 32 | 47.062 | 157.995 | 3.4× |
| 64 | 102.954 | 210.240 | 2.0× |
| 256 | 111.839 | 198.935 | 1.8× |

**This is not a seed-42 (or seed-7) artifact** — seed 42 and seed 123, run identically, show the same shape:
CP RMS at position 1 is 0.001–0.006 (vs. WDL's 250-300, ratios of 51,000×-308,000×), converging to the same
~1.8-2.0× WDL-dominant steady state by position 256 in both. **CP's gradient is not just smaller than WDL's early
in the epoch — it's negligible**, several orders of magnitude below WDL, for roughly the first 16-30 positions in
every seed tested, despite λ=0.7 nominally weighting CP more heavily in the blend. CP only becomes a comparably-sized
contributor from position ~32 onward — coinciding with where the previous experiment's stuck neurons began waking.

**Per-neuron sign consistency, seed 7 unshuffled, at position 256**: every one of the 27 non-dead L2 neurons has
`wdl_gradient_sign_consistency == 1.000` (WDL's push never once changes direction across the whole 256-position
window) vs. `cp_gradient_sign_consistency` averaging 0.439 (only 8/27 neurons reach ≥0.99). Reproduces closely on
seed 42 (23/23 neurons at 1.000 WDL vs. 2/23 for CP) and seed 123 (29/29 vs. 6/29).

**Caveat, checked rather than assumed** (motivated directly by the prior experiment's own "data order has a
material effect" finding — this decomposition could plausibly inherit the same order-dependence): re-ran seed 7
with `--shuffle-seed 102` added. The "perfect" WDL sign consistency **does not hold** under shuffling — it drops
from 27/27 neurons at 1.000 to 20/32 at ≥0.99, and the non-dead-neuron average drops from 1.000 to 0.711. This
means the unshuffled run's *perfect* consistency was inflated by the trace window (only ~256 of ~9723 positions,
drawn from a handful of early, order-fixed games with likely-correlated outcomes) — **not purely a structural
property of the WDL signal**. What *does* survive shuffling: WDL's sign consistency (0.711) still clearly exceeds
CP's (0.389) under the identical shuffled window, and the **magnitude dominance is essentially unchanged** by
shuffling (L2 `wdl_grad_rms`/`cp_grad_rms` ratio 1.82 under shuffle vs. 1.78 unshuffled at position 256; CP is still
negligible at positions 1-8 under shuffling too). So: **the magnitude finding is robust and structural; the
"perfect" directional-consistency figure is partly a data-order artifact, though a smaller, real WDL-over-CP
consistency gap survives shuffling.**

## Interpretation

CP and WDL are not fighting each other in direction on most neurons (cosine similarity between the two signals'
per-neuron gradients was mostly positive, 0.17-0.74, in the unshuffled run) — WDL is simply doing most of the work,
both because its raw magnitude is larger throughout and because, especially in the first ~16-30 positions of an
epoch, CP's gradient is close to zero regardless of seed. A plausible mechanism (not directly tested here): CP's
target (`eval_teacher`, a real-valued search-score clamp to ±600) and the freshly-initialized network's raw output
may start closer together than WDL's target (`wdl_target`, a coarse ±600/0 discrete signal) does, making CP's early
`err` small and its gradient correspondingly small, while WDL's discrete, larger-magnitude target produces a
correspondingly larger `err` and gradient from the very first position. Not confirmed — a `target_mean`/`std` vs.
early-epoch `score` comparison would test this directly, not attempted here.

For the fully-dead neurons specifically (`gradient_rms == 0.0` for the whole trace, per the prior experiment):
**both `cp_gradient_mean` and `wdl_gradient_mean` are exactly 0.0000** for these neurons in every seed — consistent
with the prior experiment's finding that these neurons are pinned in ClippedReLU's zero-gradient zone regardless of
*which* teacher signal is driving the (would-be) gradient. This decomposition doesn't change that mechanism; it
clarifies that among the neurons *not* pinned, WDL is the larger driver of wherever they end up.

## Status

- `--cp-wdl-grad-trace` stays in the codebase as an optional flag, default off, zero cost to ordinary training.
- **Not a fix, a diagnosis**: this doesn't change training behavior — it points at *where* to look next. WDL's
  outsized early-epoch influence (both magnitude, robustly, and to a lesser extent direction) is a genuinely
  actionable signal, but which remedy it implies — down-weighting WDL early (target scaling / loss weighting), a
  WDL-specific warmup, or addressing the data-order sensitivity directly (since the sign-consistency confound shows
  order and gradient-signal effects aren't fully separable) — is not decided here.
- **Sample size caveat**: 3 seeds × 1 shuffle cross-check on 1 seed. The magnitude finding replicated cleanly across
  all 3 unshuffled seeds and survived the shuffle check; the sign-consistency finding only has the one shuffle
  cross-check. Treat the magnitude conclusion as the more load-bearing of the two.
- Full per-position trace data: `cp_wdl_exp/` in scratch (3 unshuffled seeds + 1 shuffled cross-check).
