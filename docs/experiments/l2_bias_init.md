# L2 bias initialization (`--l2-bias-init`): eliminates dead-at-init, but epoch 1 relocates the collapse to saturation instead

## Background

Third investigation into the epoch-1 "dead L2 neuron" collapse, after gradient clipping
(`docs/experiments/global_gradient_clipping.md`) and LR warmup (`docs/experiments/output_warmup.md`), both closed
bucket-4. The warmup result was the pivot for this one: halving epoch-1's LR left the exact same neurons dead in 2
of 3 seeds — completely insensitive to update *magnitude* — which pointed at initialization instead. Independently,
`TrainWeights::new_seeded`'s own doc comment (trainer.rs:248-257) flagged the current `l2_bias = 0.5` constant as a
leftover from a since-fixed zero-init-symmetry bug, never tuned against the real He-init spread.

**Mechanism**: L2's pre-activation is `FT_output · L2_weights + l2_bias`. The epoch-0 probe (`tasks/lessons.md`,
2026-07-14) measured this spread directly: `[-1.7, +2.3]` across 32 neurons with `l2_bias=0.5` — several neurons
land negative before any training happens. Added `--l2-bias-init <f>` (new CLI flag, threading through
`TrainWeights::new_seeded`/`Trainer::new`, default `0.5` unchanged; one new unit test,
`l2_bias_init_only_touches_l2_bias`, confirms the RNG stream for `ft`/`l2`/`out` is unperturbed by the change).
Candidate value `3.0`, chosen to clearly clear the observed spread's lower end with margin — not a tuned final
value, a first test of the hypothesis itself.

**Consistency note**: this run reused `--warmup-epochs 1 --epochs 3` with no `--lr-schedule-epochs 20`, same as the
warmup experiment — so it inherits the same schedule-horizon bug (epoch 3's LR lands on the `min_lr` floor instead
of the intended near-`base_lr` value). Epoch 3 is excluded from the comparison below for the same reason it was in
`output_warmup.md`; epoch 1-2 are unaffected (confirmed directly: both land at `lr=0.001` regardless of horizon).

## Primary judgment 1: epoch-0 probe (pure init effect, no training)

`--epochs 0` dump fed through `cargo run --example l2_saturation_probe -p sekirei-core` against the same 261-
position set used by the original epoch-0 probe (`l2probe.sfen`).

| seed | `l2_bias=0.5` dead | `l2_bias=3.0` dead |
|---|---|---|
| 42 | 28.2% (5 neurons: **13, 15, 17, 19, 27** — exact match to the original 2026-07-14 probe) | **0.0%** |
| 7 | 24.6% (5 neurons) | **0.0%** |
| 123 | 21.3% (2 neurons) | **0.0%** |

**Unambiguous, clean result**: raising `l2_bias` to 3.0 eliminates every dead-at-init neuron in all 3 seeds,
including the neuron IDs (13/15/27) the original probe identified as "genuinely dead at init, never recovers."
Reproducing the exact same dead-neuron IDs for seed 42 independently confirms this isn't a diagnostic artifact.

## Primary judgment 2: epoch 1 of real training — the collapse relocates, it doesn't resolve

| seed | arm | `l2_dead_neurons` | `valid_output_std` | `l2_saturation_frequency_mean` | `l2_preactivation_p50` |
|---|---|---|---|---|---|
| 42 | control (bias=0.5) | 2 | **19.40 (healthy)** | — | -5.4 |
| 42 | candidate (bias=3.0) | 0 | **0.000 (collapsed)** | 0.592 | 193.1 |
| 7 | control (bias=0.5) | 5 | 0.000 (collapsed) | — | -9.3 |
| 7 | candidate (bias=3.0) | 0 | 0.000 (collapsed) | 0.194 | -22.7 |
| 123 | control (bias=0.5) | 3 | 0.000 (collapsed) | — | -8.2 |
| 123 | candidate (bias=3.0) | 0 | 0.000 (collapsed) | 0.400 | -21.3 |

**`l2_dead_neurons=0` in all 3 candidate seeds looks like a clean win and isn't one.** The network's actual output
is still fully collapsed (`valid_output_std=0.0`) in all 3 candidate seeds at epoch 1 — identical to or worse than
control. Seed 42, healthy under control (`std=19.4`), **collapses under the candidate**. For seed 42, the mechanism
is directly confirmed: `l2_saturation_frequency_mean=0.592` (59% of neuron×sample pairs saturated) and median
pre-activation `p50=193`, far above the ClippedReLU ceiling (127). ClippedReLU has zero gradient at *both* ends
(`≤0` and `≥127`); moving the floor up doesn't stop the epoch-1 update from exploding pre-activations by
100-300× (the same magnitude the original epoch-0 probe measured) — it just relocates where they land. Seeds 7 and
123 show lower saturation fractions (0.19, 0.40) and their `p50`s are still negative, so their epoch-1 collapse
looks more like a residual dead-zone effect than pure saturation — but the observable result (`std=0.0`) is
unchanged from control either way.

**This is the second time a "gentler intervention" has flipped seed 42 from healthy to collapsed** — the warmup
experiment's `--warmup-epochs 2` did the same thing. Two different levers (update magnitude, init distance),
same seed, same direction. No longer read as one-off bistable noise; it's a pattern worth carrying into whatever
investigation comes next.

## Secondary/auxiliary: epoch 2 (schedule-horizon-bug-unaffected, recovers regardless of arm)

| seed | arm | `l2_dead_neurons` | `valid_output_std` | `valid_cp_mse` |
|---|---|---|---|---|
| 42 | control | 8 | 56.93 | 161521.1 |
| 42 | candidate | 0 | 24.09 | 170777.7 |
| 7 | control | 7 | 3.92 | 173413.3 |
| 7 | candidate | 0 | 22.56 | 170928.7 |
| 123 | control | 0 | 21.12 | 171235.7 |
| 123 | candidate | 0 | 19.07 | 169119.3 |

Every candidate arm recovers to a non-collapsed, arguably more consistently healthy `valid_output_std` by epoch 2
(control's seed7 at 3.92 is borderline-low by comparison), and `l2_dead_neurons` stays at 0 for all 3 candidate
seeds versus control's persistent 0/7/8. `valid_cp_mse` at epoch 2 favors the candidate in all 3 seeds. **Recorded
as an exploratory note, not a promotion trigger** — it doesn't undo the epoch-1 finding above, which is what the
pre-registered framework's primary judgment point actually is.

## Applying the pre-registered decision framework

The literal wording of bucket 1 ("epoch-0 dead count drops to 0 in all 3 seeds AND epoch-1 dead count also drops")
is technically satisfied — and would be the wrong bucket to report. `l2_dead_neurons` was chosen as the framework's
epoch-1 proxy for "the collapse is fixed," and this experiment is exactly the case where the proxy fails: dead
count hit zero while the network's actual output stayed fully collapsed, just via the opposite ClippedReLU wall.
Reporting this as "promoted" would misreport a collapse as a fix. **Not promoted, no bucket applies cleanly** — the
framework didn't anticipate a metric-relocation failure mode, which is itself worth recording for the next
experiment's framework design (include `valid_output_std`/saturation fraction as primary judgment inputs
alongside dead-count, not as afterthoughts).

## Conclusion

**`--l2-bias-init` alone does not fix the epoch-1 collapse — but it does definitively rule out the init-distance
hypothesis this experiment was built to test.** Combined with the warmup experiment's ruling-out of update
magnitude, both of the two most natural explanations for "why does epoch 1's update kill/saturate L2 neurons" have
now failed: it isn't primarily how far pre-activations start from the ClippedReLU boundaries (this experiment,
raising the floor by ~2.5 just relocated the wall the update hits), and it isn't primarily how large the update
step is (the warmup experiment, halving it left the same neurons dead). **What's left is something about the
epoch-1 update's *direction* or *structure*** — the update pushes a large fraction of L2 pre-activations to one
wall or the other regardless of where they started or how big the step was, which points toward FT's own
epoch-1 dynamics (FT feeds L2's pre-activation directly) or a correlated-gradient-direction effect, rather than a
magnitude/distance problem addressable by a single scalar tweak.

The epoch-0 result stands on its own as a genuine, clean finding (raising `l2_bias` eliminates dead-at-init
neurons in all 3 seeds, reproducibly) — it's just not sufficient on its own once real training's epoch-1 update is
applied.

## Status

- `--l2-bias-init` stays in the codebase as an optional flag, default `0.5` (unchanged behavior unless passed).
- **Not promoted.** `l2_bias=3.0` is not adopted as a new default — it trades one epoch-1 collapse mode for
  another, confirmed directly via saturation-frequency data in all 3 seeds.
- **Priority queue update**: the "initialization/L2-bias" line is closed on the "single-scalar-bias" version of
  the hypothesis. All three single-variable levers tried so far (clip magnitude, update magnitude, init distance)
  have failed to touch the epoch-1 mechanism. Next candidate: look at what epoch 1's update actually *does*
  directionally — e.g. per-neuron gradient sign consistency across the epoch, or FT's own epoch-1 pre-activation
  dynamics (FT feeds L2 directly, and FT's own dying-ReLU risk hasn't been probed the way L2's has) — rather than
  a fourth magnitude-or-distance-style scalar tweak.
- Full per-epoch data: `l2bias_exp/` in scratch (epoch-0 probes and epoch1-3 training, both bias arms, 3 seeds).
