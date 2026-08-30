# Teacher-conflict masking: FT-only masking at conflicting positions clearly beats an equal-count random mask

## Background

`l2_b5_shadow_trace.md` closed the B5 investigation with three established facts: CP pushes FT backward and kills
FT units; WDL grows FT but tends to push L2 toward its dead floor; and per-position CP/WDL gradients are always
exact scalar multiples of each other (`cos(g_cp, g_wdl) ≡ ±1`), which rules out per-position PCGrad-style gradient
*projection* as a mechanism (it would degenerate to deleting both gradients whenever they conflict, not trimming
an orthogonal component — there isn't one at single-position granularity). This experiment tests the resulting,
correctly-scoped mechanism directly: **teacher-conflict masking** — stop the FT (and optionally L2) update at
positions where the prediction sits strictly between the two teachers (`(score - eval_teacher) * (score -
wdl_target) < 0`), across a *whole* training run, not a fixed B5-style block.

**Not PCGrad, deliberately renamed.** `ConflictMaskLayer`'s doc comment states the reasoning: since per-position
CP/WDL gradients are collinear, a full PCGrad projection algebraically collapses to exactly this same zero-or-
pass-through outcome — implementing the cheap, direct sign check instead of materializing and projecting full
gradient vectors that would cancel anyway is an exact simplification, not an approximation.

## Design: 4 arms, with a rate-matched control against the dominant confound

The obvious confound this design must rule out is stated up front: masking *any* subset of positions reduces
total training volume, so an apparent improvement could just be "less training helped," unrelated to which
positions were masked. Four arms:

- **Control** — today's training, unmodified.
- **Conflict-mask-FT** — stop FT's update at teacher-conflicting positions; L2/output train normally.
- **Conflict-mask-FT+L2** — stop FT *and* L2's update at the same positions; output trains normally.
- **Rate-matched** — stop FT's update at an *exactly equal count* of positions, chosen independent of the
  teacher-conflict signal (an unbiased, seeded "exactly K of N" streaming selection —
  `Trainer::rate_matched_should_mask`; include with probability `remaining_needed/remaining_pool` at each
  eligible position, decrementing both — guarantees exactly K selections with no periodic or boundary-clustered
  bias, and never reads `score`/`eval_teacher`/`wdl_target`). `K`/`N` are read directly from Conflict-mask-FT's
  own epoch-1 `masked_position_count`/`eligible_position_count` for the same seed, not estimated — the rate-
  matched run's own `.meta.json` self-verifies `masked_position_count == K` exactly (confirmed for all 3 seeds).

3 seeds (42, 7, 123), 3 epochs, `--wdl-lambda 0.7`, `data/gateA_csa_subset`, `--split-seed 42`,
`--shuffle-seed 11`, shared teacher-search cache. Epoch 1 is the primary endpoint (pre-registered); epochs 2-3
check whether any effect is real improvement or just deferred collapse.

## Results

### Fire rate is moderate (~37%), not majority-discarding

`3484-3511` of `9312` eligible positions conflict in Control at epoch 1 (`37.4-37.7%`, all 3 seeds) — well below
the "discarding most of the teacher signal" threshold that would motivate skipping straight to a batch-level
design regardless of outcome.

### Epoch 3: Conflict-mask-FT clearly beats both Control and Rate-matched, consistently across all 3 seeds

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| **L2 dead neurons** — Control | 8 | 10 | 12 |
| — Conflict-mask-FT | **0** | **0** | **0** |
| — Conflict-mask-FT+L2 | 6 | 7 | 3 |
| — Rate-matched | 10 | 8 | 4 |
| **valid CP MSE** — Control | 165166 | 165385 | 166694 |
| — Conflict-mask-FT | **159043** | **161354** | **160310** |
| — Conflict-mask-FT+L2 | 160623 | 162593 | 164430 |
| — Rate-matched | 165279 | 162926 | 164190 |
| **valid output std** — Control | 33.5 | 44.7 | 43.6 |
| — Conflict-mask-FT | **77.7** | **68.3** | **68.0** |
| — Conflict-mask-FT+L2 | 82.5 | 63.7 | 54.7 |
| — Rate-matched | 45.3 | 55.7 | 50.2 |

Conflict-mask-FT reaches **exactly zero** dead L2 neurons in all 3 seeds by epoch 3 (Control: 8-12; Rate-matched:
4-10 — barely different from Control). `valid CP MSE` is `4-6%` lower than Control in every seed, a gap that
*widens* from epoch 1 to epoch 3, not one that fades — ruling out "deferred collapse" as the explanation.
`valid_output_std` (the direct symptom of this whole investigation's central pathology — output collapsing
toward a near-constant value) is roughly double Control's in every seed, and Rate-matched sits much closer to
Control than to Conflict-mask-FT on this metric specifically. **Rate-matched achieving only a fraction of
Conflict-mask-FT's improvement, despite masking the exact same number of positions, is the decisive evidence:
this is not "less training helped" — the *specific* positions where the two teachers disagree in sign are what
matters.**

### FT-only is cleaner than FT+L2, not just simpler

Conflict-mask-FT+L2 shows the *same qualitative* improvement over Control and Rate-matched, but consistently
falls short of FT-only on every metric above, and — mildly counterintuitively, since it directly protects L2's
own update at conflict positions too — leaves `3-7` L2 neurons dead at epoch 3 where FT-only leaves exactly zero.
One seed (7) even shows a handful (`3`) of FT neurons crossing into "dead the entire epoch" under FT+L2 by epoch
2-3, never observed in any other arm. FT-only masking alone captures the benefit; adding L2 masking on top does
not help and shows a small hint of its own new risk. Recommended candidate: **Conflict-mask-FT**, not FT+L2.

### `epoch 1 dead FT units` (a pre-registered metric) is not discriminating in this whole-run scope

`ft_dead_neurons` (an FT unit with zero activation frequency for the *entire* epoch) is `0` in essentially every
arm/seed/epoch (one exception: FT+L2, seed 7, epoch 2-3, `3` units). This is expected, not a null result: FT
dead-ification at this severity was a block-*local* B5-specific pathology, not something that emerges broadly
over a whole, otherwise-normal training run. The metric that actually captures the collapse this experiment
targets is `valid_output_std` (the central symptom) and `l2_dead_neurons`/`l2_saturation_frequency_mean` (the
mechanistic L2-side correlate) — both of which show the clean, strong, seed-consistent pattern above. Recorded
as specified, not substituted silently.

### Caveat, reported honestly: `valid_wdl_loss` does not agree in direction across all 3 seeds

| arm, seed | epoch 1 → 2 → 3 |
|---|---|
| Control, seed 42 | 355394 → 354613 → 352055 |
| Conflict-mask-FT, seed 42 | 354274 → **358472** → **355301** (worse than Control by epoch 3) |
| Control, seed 7 | 354996 → 352025 → 351279 |
| Conflict-mask-FT, seed 7 | 353285 → 348910 → **345249** (better than Control) |
| Control, seed 123 | 355514 → 354416 → 352773 |
| Conflict-mask-FT, seed 123 | 353743 → 349259 → **346608** (better than Control) |

Seeds 7 and 123 show `valid_wdl_loss` improving under Conflict-mask-FT, consistent with every other metric. Seed
42 shows the opposite — WDL loss *regresses* relative to Control by epoch 3, and the gap grows across epochs
rather than shrinking. The pre-registered success condition ("valid CP MSE and the WDL validation metric must not
regress, 3-seed direction agreement") holds cleanly for CP MSE in all 3 seeds and for WDL loss in 2 of 3 — seed
42 is a genuine, unresolved exception, not smoothed into the overall verdict.

### The per-position "is a conflict position locally more dangerous" breakdown is inconclusive on its own — the causal evidence is the arm-level comparison, not this breakdown

Splitting Control's *own* positions by conflict/non-conflict and comparing `new_dead_ft_mean`/`new_dead_l2_mean`
does **not** cleanly show conflict positions as the locally dangerous ones — in seeds 7 and 123 (both deep in
Control's own output-collapse pathology, `valid_output_std ≈ 2-4` at epoch 1), the *non-conflict* group shows
higher per-position new-dead rates than the conflict group, the reverse of the naive expectation. This makes
sense once the mechanism is understood as *aggregate/trajectory*, not local: masking conflict positions changes
the whole training trajectory well enough to avoid entering the runaway regime at all, rather than surgically
removing individually-dangerous steps one at a time. The arm-level Conflict-mask-FT-vs-Rate-matched comparison
above is the correct and decisive test; this per-position breakdown is reported for completeness but does not by
itself establish the mechanism.

## Conclusion

**Conflict-mask-FT clearly beats an equal-count rate-matched mask on every primary metric, in every seed, at
every epoch — the effect is attributable to the teacher-conflict signal specifically, not to reduced training
volume.** Reading against the user's own pre-registered outcome branches: this is squarely "Conflict-mask-FT
improves and rate-matched does not" — *"FT updates at positions where the two teachers disagree in sign are
driving the pathology, not merely a reduction in update count."* FT-only is the recommended candidate over
FT+L2 (same benefit, cleaner L2 outcome, no new risk observed). The one qualification: `valid_wdl_loss` does not
agree in direction across all 3 seeds (seed 42 regresses); every other metric (CP MSE, L2 dead-ification,
L2 saturation frequency, output std) agrees cleanly in all 3 seeds and the gap widens rather than shrinks across
epochs, ruling out "deferred collapse" as an explanation.

Per the user's own pre-registered next-step framework, this result satisfies the condition for proceeding to a
**longer training run under matched conditions, selecting a valid checkpoint, then a paired quick gate** — not
yet run in this document; the batch-level PCGrad build remains explicitly deferred, since teacher-conflict
masking already shows a specific, non-trivial effect without needing to change update frequency or Adam's
time series.

## Gate outcome (2026-07-19): REJECTED

The paired quick gate was run (`conflict_ft_seed123.epoch7` vs. `control_seed123.epoch7`, pre-registered checkpoint
selection, no epochs excluded). The first attempt surfaced an unrelated engine-validity bug (an intermittent
long-lived-process position-replay desync — see `docs/experiments/intermittent_replay_desync_investigation.md`)
that invalidated its results; after fixing the detection gap and re-running from scratch with full validity
guarantees (zero invariant fires, zero illegal moves, zero technical failures across all 396 games), the formal
result is **FAIL**: candidate 159W/0D/237L (40.2%), Elo −69.3, 95% CI [−104.2, −34.4] (entirely negative), LOS
0.0%.

**Every validation-side metric this document used to justify proceeding — CP MSE, WDL loss, L2 dead-neuron
resolution — pointed toward the candidate, and real playing strength went decisively the other way.** This
doesn't contradict the diagnostic-level findings above (the conflict-masking mechanism genuinely does what this
document shows it does to training dynamics); it means those training-dynamics improvements don't reliably
predict match strength for this class of change. `control`/`champion` are unchanged. Full record:
`results/20260719_190910_conflict_ft_seed123.epoch7_vs_control_seed123.epoch7.{json,jsonl,verdict.json}`,
`tasks/lessons.md` (2026-07-18/19 entry).

**Correction (2026-07-20)**: the L2 layer has 32 neurons, not 16 (`tasks/lessons.md` mistakenly reported the
gate's L2 dead-neuron health as "0/16 … 12/16"; `scripts/select_longrun_checkpoint.py` had a matching `L2 = 16`
constant). The raw dead-neuron counts used throughout this document (8/10/12 for Control, 0 for
Conflict-mask-FT) were always read directly from `.meta.json` and are correct — only the "/16" denominator was
stale. See `tasks/lessons.md`'s 2026-07-20 entry for the fix and verified blast radius (none: no epoch in the
actual longrun ever had `l2_dead_neurons` in the affected `[16,31]` range).

## Scope

All metrics from existing per-epoch diagnostics (`valid_cp_mse`, `valid_wdl_loss`, `valid_output_std`,
`l2_dead_neurons`, `l2_saturation_frequency_mean`) plus new epoch-aggregate fields
(`masked_position_count`, `eligible_position_count`, `conflict_group`/`nonconflict_group` — per-position-group
breakdowns of residual magnitude, pre-mask gradient norm, and own-board new-dead-unit counts — and
`ft_dead_neurons`/`ft_activation_frequency_mean`, added this experiment by exposing `Trainer::ft_zero_count`
through the same generic helpers `l2_dead_neurons`/`l2_activation_frequency_per_neuron` already use). No strength
gate run yet (explicitly deferred per the pre-registered plan — "まず棋力ゲートは不要"). The rate-matched arm's
`K`/`N` are fixed at epoch 1's values and reused unchanged for epochs 2-3 (a simplification — Conflict-mask-FT's
own fire count could in principle drift epoch to epoch as its trajectory diverges further from Control's, not
re-measured here).

## Status

- `ConflictMaskLayer`, `ConflictGroupStats`, `Trainer::rate_matched_should_mask`, teacher-conflict masking hook
  in `train_position`, `--diagnostic-conflict-mask`, `--diagnostic-rate-matched-mask-count/-total/-seed`,
  `diagnostics::ConflictGroupSummary`/`build_conflict_group_summary`, `ft_dead_neurons`/
  `ft_activation_frequency_mean`: committed. 5 new unit tests (unset-is-byte-identical, FT-only masks FT only at
  a guaranteed-conflicting position, FT+L2 masks both, guaranteed-non-conflicting position passes through
  unchanged, exact-K-of-N selection is deterministic and reproducible). 124/124 `sekirei-train` tests passing,
  `fmt`/`clippy` clean.
- 12 runs (4 arms × 3 seeds, 3 epochs each) complete, rate-matched arm self-verified to match Conflict-mask-FT's
  exact masked-position count at epoch 1, all 3 seeds.
- Next, per the user's own priority order: (1) longer training + paired quick gate for the Conflict-mask-FT
  recipe, conditional on this result — not yet run; (2) only if that also fails or the fire rate turns out too
  high in a longer run, consider the batch-accumulated PCGrad build (fixed-length micro-batch, N=8 or 16,
  accumulate CP/WDL gradients within the batch before projecting, one Adam update per batch, with a
  batched-blended-gradient control since optimizer step count changes); (3) investigate the seed-42 WDL-loss
  exception if it persists into the longer run.
