# Sample-level gradient correlation trace (`--sample-grad-trace`): Stage 1 built and verified; Stage 2's frozen-weights screen can't settle the order question, but surfaces a sharper finding

## Background

`docs/experiments/cp_wdl_gradient_decomposition.md` established that WDL's per-neuron gradient sign consistency is
high (often exactly `1.000`) among non-dead L2 neurons — a specific neuron, whenever it receives WDL-driven
gradient, almost always receives it in the same direction. The working hypothesis for *why*: `wdl_target` is
constant within a game, so consecutive same-game positions push a neuron the same way, and if training visits many
same-game positions consecutively, that one-directional push could accumulate faster than any opposing signal can
correct it — potentially driving a neuron past a ClippedReLU wall. Per explicit scope-down instruction, this was
split into two stages before touching the production training pipeline: **Stage 1** — record per-position gradient
correlation data without changing training order at all; **Stage 2** — reorder the *recorded* data offline
(original / game-shuffled / sample-shuffled / outcome-balanced) and see whether any reordering would plausibly
reduce one-directional accumulation, entirely as a fixed-weights counterfactual, before deciding whether a Stage 3
micro-replay harness (real training, order as the only variable) is worth building.

## Stage 1: implementation

`wdl_target_cp`-style parameter threading, not a new derivation: `train_game` gained a `game_id: u64` parameter
(the game's stable index into the caller's game list, independent of `--shuffle-seed`); `train_position` gained
`game_id`/`game_result`, feeding a new `--sample-grad-trace <n>` flag (default `0`, off) that records one
`SampleGradRecord` per position, up to `n` positions per epoch: `game_id`, `game_result`, `position_index`,
`prediction`, `cp_target`/`wdl_target`, `cp_d_output`/`wdl_d_output` (cheap scalars, no full `diagnostic_backward`
call needed), the real blended `l2_grad_vector` (`d_l2_acc`, the same 32-wide vector `train_position`'s own
backward pass computes) and its norm, cosine similarity vs. the previous recorded position and vs. the running
mean, and a per-neuron gate state (`dead`/`linear`/`saturated`). Writes `<output>.epochN.sample_grad.jsonl`.
**Never reorders training** — this is Stage 1's entire point.

**Verification, all required before trusting the trace**:
- 97 tests pass (3 new: two `cosine_similarity`/`vector_cosine_similarity` helper tests plus record-content and
  identity tests), `fmt`/`clippy` clean.
- **Decomposition identity**: `dL/dL2_preactivation = dL/dOutput × output_weight × ClippedReLU'` (the exact
  formula `train_position`'s own backward pass already uses) reconstructed independently in a dedicated unit test
  from `forward()`'s own pre-update score and pre-update `out` weights, checked against `l2_dacc_sum` (which after
  one call is exactly that call's `d_l2_acc`) — passes for every neuron.
- **State non-alteration**: enabling `--sample-grad-trace` produces byte-identical trained weights, `total_loss`,
  and gradient accumulators to the flag being off, both in a dedicated unit test and in a live comparison run
  (`--sample-grad-trace 256` vs. omitted, same seed/recipe) — byte-identical `.bin`, identical `.meta.json`.

## Stage 2: offline reordering analysis

### Primary bucketing: inconclusive, not by design flaw but by a real, separate finding

The original plan: bucket adjacent-pair `l2_grad_vector` cosine similarity by same-game / cross-game-same-outcome /
cross-game-different-outcome, directly in the recorded (unshuffled) order. Result (seed 7, 1024 positions, 30
distinct games spanning `BlackWin`/`WhiteWin`/`Draw`):

| bucket | n | mean cosine |
|---|---|---|
| same_game | 135 | +0.848 |
| cross_game_same_outcome | 3 | +0.286 |
| cross_game_different_outcome | **0** | — |

`cross_game_different_outcome = 0` even across 1024 positions and a real outcome mix at the *game* level (games
visited alternate `BlackWin`/`WhiteWin`/`Draw` freely — verified directly, not sorted by outcome). The cause isn't
data sparsity at the game level; it's this:

**Headline finding — L2's gradient collapses to exactly zero for almost every position by ~1/4 of the way into
epoch 1.** A position's `l2_grad_vector` is entirely zero when every one of 32 neurons is outside `(0, 127)` (dead
or saturated) simultaneously. Measured directly (confirmed non-artifactual: `l2_grad_vector` all-zero and `l2_gate`
all-nonzero agree on every single record, 0 mismatches):

| quartile (of first 1024 positions) | all-zero-gradient rate |
|---|---|
| 1 (positions 1–256) | 45.7% |
| 2 (positions 257–512) | 99.2% |
| 3 (positions 513–768) | 99.6% |
| 4 (positions 769–1024) | 100.0% |

By position ~300, essentially **every** position pushes zero gradient to L2 — not most, not the majority, all of
them. Since the primary bucketing only counts pairs where *both* positions have nonzero gradient, and >99% of the
window is gradient-dead from the second quartile onward, there's structurally almost no data left to populate a
fair cross-game contrast — this is a real, separate, and probably more important finding than the bucketing it
broke. It sharpens (doesn't just restate) the existing output-scale-runaway line: it's not merely that output
scale grows unboundedly, but that the growth apparently gates the *entire* L2 layer shut for the rest of the
epoch almost immediately.

### Secondary: per-neuron prefix-drift under reordering — corrected methodology, but scoped to survivors only

First attempt compared one realization each of game-shuffled/sample-shuffled/outcome-balanced against the original
order and found the prefix-drift statistic (`max_k|running sum| / Σ|g|`, a parameter-free, order-sensitive
measure of one-directional accumulation) **identical to several decimal places** across all three reorderings.
This looked like clean evidence that reordering doesn't matter — **it wasn't**. Diagnosed before writing it up:
for a neuron whose active-sample signs are heavily imbalanced (a real example: 103 negative vs. 21 positive
samples), a *single* random shuffle realization lands within ~0.3% of the metric's theoretical floor almost every
time, because true overshoot requires *adversarially clustering* same-sign values together, which one random draw
essentially never produces by chance (verified directly: 1000 random permutations of that exact neuron's values
ranged only 86.07–88.91, vs. a true adversarial-clustering ceiling of 99.51). Comparing one shuffle to the original
order was comparing two draws from a narrow distribution, not detecting order-invariance.

**Corrected**: for each "contested" neuron (≥20 nonzero-gradient samples in the window — genuinely evaluable, not
one of the near-permanently-dead ones), computed 200 random trials each for game-shuffled and sample-shuffled
orderings, and located the *actual recorded order's* drift as a percentile within that combined distribution.
Result, 3 seeds, all consistent:

| seed | contested neurons | mean percentile | median percentile |
|---|---|---|---|
| 7 | 10 | 41.2% | 37.9% |
| 42 | 14 | 32.1% | 30.1% |
| 123 | 12 | 34.0% | 36.5% |

All three land **below the 50th percentile** — the actual training order's one-directional accumulation, among
neurons that are actively receiving gradient, already looks *at least as good as* a typical random reordering, not
like an adversarial outlier. Taken at face value, this says reordering offers no exploitable headroom to reduce
accumulation for these neurons.

**Critical limitation, caught before drawing that conclusion (this is the load-bearing caveat of this whole
doc)**: "contested" neurons are, by construction, exactly the ones that *didn't* die — a dead neuron's `l2_grad_
vector` is mostly/entirely zero (that's what dead means), so it never clears the ≥20-nonzero-sample bar and is
excluded from this analysis entirely. **This screen only asks "does reordering help already-surviving neurons
accumulate less," and is structurally blind to "does reordering change which neurons die in the first place."**
Those are different questions, and only the second is what the original hypothesis was actually about.

### This directly conflicts with an existing, real-training finding — reconciled, not glossed over

`docs/experiments/epoch1_batch_trace.md`'s Stage 2 (`--shuffle-seed`, **real training**, not a frozen-weights
counterfactual) found exactly the death-formation sensitivity this Stage 2 screen can't see: with identical
initialization and identical data, 2 of 3 shuffle seeds reached **zero** dead L2 neurons by position 256, an
outcome no fixed-order run ever reached — and traced it to specific neurons "waking" (starting to receive nonzero
gradient) at different points depending purely on which position happened to land early in the shuffled sequence.
That is a real, already-established order effect on the death/freeze mechanism itself.

**This doc's Stage 2 result does not contradict that finding — it simply never tested the same question.** The
survivor-only prefix-drift screen and the wake-up/freeze mechanism are different mechanisms operating on different
neuron populations (already-active vs. already-dead-or-about-to-die). Reporting "reordering doesn't help" from
this screen alone, without this reconciliation, would have been the same class of overclaim already caught and
corrected twice earlier in this investigation line (the "order beats initialization" and "WDL target is bad"
walk-backs) — this time caught before writing the doc, via an advisor review, rather than after.

## Conclusion

**Game order affects dead-neuron formation — existing evidence already shows this** (`epoch1_batch_trace.md`'s
`--shuffle-seed` result: 2 of 3 shuffle seeds reached zero dead L2 neurons where no fixed-order run did). **The
offline, fixed-weights reordering analysis in this doc cannot judge the effect of actually changing training
order** — it's a counterfactual over already-recorded gradients, not a re-run. **This new analysis only evaluates
already-surviving neurons' cumulative gradient, so the original hypothesis (does reordering change which neurons
die) remains undecided.**

This is why the two next threads are sequenced, not left open: (1) the 86%→100% all-zero-gradient collapse
(measured directly above, position ~300 onward) is investigated first, since if the L2 gradient path is closed
entirely by then, neither data-order changes nor loss-weighting changes downstream of that point can do anything —
next up is classifying *why* each zero occurred (residual-zero / output-path-zero / clamped-low / clamped-high)
per position band, followed by a forward-side decomposition of what actually moves `L2`'s pre-activation
(`Δz = Δh·W_old + h_old·ΔW + Δh·ΔW + Δb`) around the collapse point. (2) Only after that structural question is
answered does a multi-seed `--shuffle-seed` sweep (reusing the existing flag, no new sampler) test how far order
can move the point/manner of collapse. Separate write-ups once each is run.

## Status

- `--sample-grad-trace` stays in the codebase as an optional flag, default `0` (off, zero added cost).
- Stage 1 (recording): complete, verified, no training-behavior change.
- Stage 2 (offline reordering analysis): complete as a *screen*, but its negative result on survivors does not
  extend to a claim about order and death formation — see the reconciliation above.
- Stage 3 (micro-replay harness): **not started**, and not clearly justified by this doc's own Stage 2 result
  alone — see "Conclusion."
- Analysis script and raw data: `sample_grad_exp/` in scratch (`analyze_stage2.py`, 3-seed 256-position runs, one
  1024-position seed-7 run for the wider primary-bucketing check).
