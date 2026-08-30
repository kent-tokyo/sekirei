# Stage C: B2xB7 interaction — positive main effect, negative marginal interaction, no priming

## Background

Stage B (`l2_saturation_ft_freeze_block_screen_stage_b.md`) replicated B2 as a necessity-candidate block and B7 as
a sufficiency-candidate block at 3 seeds, but ranked them by *separate* single-block tests — it could not say
whether B2's presence changes what B7 accomplishes. This experiment tests that directly with a 2x2 design: within
the `16-271` window, every block except B2 (`48-79`) and B7 (`208-239`) stays frozen throughout; B2 and B7 are each
independently toggled active/frozen.

**Design**: `Neither` (both frozen — identical to the existing `all-frozen` arm), `B2 only` (identical to Stage
A/B's `single_active_B2`), `B7 only` (identical to `single_active_B7`), `Both` (both active — new, requires
reactivating two disjoint blocks at once). New `--diagnostic-ft-reactivate2-from-position`/`--diagnostic-ft-
reactivate2-until-position` flags add a second, independent reactivation window (OR'd with the first; default
`0`/`0`, byte-identical to the mechanism not existing — `l2_sample_count` is always `>=1`). 112/112 `sekirei-train`
tests pass (2 new: two disjoint reactivation holes both open correctly; byte-identical to a single window when the
second is left unset), `fmt`/`clippy` clean. `3` seeds (`42`/`7`/`123`); only `Both` required new training (`3`
runs) — `Neither`/`B2 only`/`B7 only` reused from Stage A/B with exact metadata match.

**Validity check before trusting anything**: at position `192` (strictly before B7's block starts), `Neither` and
`B7 only` must be identical, and `B2 only` and `Both` must be identical — confirmed to the reported decimal in all
3 seeds (`diff=0.00` throughout). Arm construction and checkpoint attribution are sound.

## Results

### Final state at position 271 (= position 240, since B8 is frozen in every arm here)

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| Neither | 14.09 | 13.75 | 13.77 |
| B2 only | 28.79 | 27.55 | 28.21 |
| B7 only | 47.07 | 45.39 | 46.02 |
| **Both** | **59.66** | **58.78** | **59.58** |

`Both > B7 only > B2 only > Neither` in every seed, on `‖h‖@271`.

### Positive priming is not supported on any metric, in any seed

`‖h‖`, weighted input (both pooled-over-probe-set and restricted to the same single board/saturating-neuron
population `‖h‖` uses), and raw FT parameter movement over B7's own block window (`208→240`) were all checked.
None showed `Both` exceeding the additive prediction (`B2 only + B7 only − Neither`) in any seed. The hypothesis
that B2 primes or amplifies B7's effect is not supported.

### But the picture is not simply additive either — B2 measurably dampens B7's own marginal contribution

**On `‖h‖`, the interaction looks small**: `Both − B2only − B7only + Neither` = `-0.4` to `-2.1` across seeds
(`1-7%` of B7's own standalone effect, `B7only − Neither ≈ 31-33`).

**On weighted input (z, the quantity directly compared against the `127` saturation threshold), the interaction
is clearly negative and much larger**, checked two ways to rule out an aggregation artifact:
- Pooled over the full 261-position probe set: interaction `-6.1` to `-13.3` (`22-27%` of B7's own effect).
- Restricted to the same single board `‖h‖` uses, saturating neurons only: interaction `-20.6` to `-27.5`
  (`20-26%` of B7's own effect).

Both z-based measurements agree closely (`20-27%`) and are not sensitive to whether dead/linear neurons across
the full probe set are included — the negative interaction is not a dilution artifact of pooling.

**B7's own raw FT parameter movement during its own block (position `208→240`) is consistently smaller when B2
has already been active**:

| seed | `‖Δθ_FT‖` B7-only | `‖Δθ_FT‖` Both | ratio |
|---|---|---|---|
| 42 | 644.4 | 592.7 | 0.920 |
| 7 | 630.7 | 594.9 | 0.943 |
| 123 | 635.7 | 595.3 | 0.936 |

A `6-8%` reduction, all 3 seeds, same direction. **This is evidence at the level of the raw parameter update
itself, independent of any norm or aggregation choice** — it rules out "the negative interaction is just an
artifact of how `‖h‖` or `z` are aggregated." The dampening is real at the mechanism level: B7's own block
receives (or produces) less parameter movement when B2 has already moved FT.

## Interpretation

**B2 does not prime B7 — it has a positive main effect on FT norm and weighted input on its own, while
simultaneously, independently, dampening B7's marginal contribution when both are active.** `Both` ends up largest
not because of positive synergy, but because B2's own direct additive contribution exceeds the size of the
negative interaction it produces on B7's marginal effect. This is neither "purely additive" nor "strong
interference" — it is **a positive main effect plus a negative marginal interaction**, and both parts are real and
separately supported.

**Why `‖h‖` and weighted input disagree in magnitude is not a contradiction — they measure different things.**
`‖h‖` is the overall size of the FT representation; weighted input is that representation's component projected
onto the specific L2 row directions that the `127` threshold actually gates on. If B2 grows `‖h‖` in a direction
that only partially overlaps with the direction B7's own contribution uses, `‖h‖` can add close to linearly (the
overall size grows roughly additively) while the *useful* (threshold-relevant) component shows a real negative
interaction. This reconciliation is not chased further here — the `‖h‖`-vs-weighted-input divergence itself is not
pursued as its own question at this time, per priority ordering below.

## Scope

Same standing caveats as Stage A/B (necessity/sufficiency budgets differ; data content and trajectory state remain
entangled). The mechanistic explanation above (directional overlap of B2's and B7's respective `Δh` contributions)
is a plausible reconciliation of the `‖h‖`-vs-weighted-input gap, not independently verified by a direct
angle/projection measurement in this experiment — deferred.

## Status

- `--diagnostic-ft-reactivate2-from-position`/`--diagnostic-ft-reactivate2-until-position`: implemented, tested
  (112/112 `sekirei-train` tests pass, 2 new), `fmt`/`clippy` clean. Committed.
- 3 new runs (`Both`, seeds 42/7/123), complete, self-verified; `Neither`/`B2 only`/`B7 only` reused from Stage
  A/B. Raw data and scripts in scratch (`ft_block_interaction_exp/`: `run.sh`, `verify_meta_both.py`,
  `satprobe_out/`, `xcross_out/`).
- Next: B5's anti-growth mechanism (radial/orthogonal `Δh` decomposition around its `144-175` block), then, if
  confirmed, a B5↔B7 ordering intervention. B4's seed-dependent local-firing divergence follows after that.
