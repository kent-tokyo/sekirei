# Why CP and WDL gradient scales differ: target coarseness, not loss weighting

## Background

`--cp-wdl-grad-trace` (`docs/experiments/cp_wdl_gradient_decomposition.md`) established that WDL's gradient
magnitude dominates CP's — negligibly small CP contribution for the first ~16-30 positions of an epoch, converging
to a ~1.8-2.0× WDL-dominant steady state by position 256 — despite `--wdl-lambda 0.7` nominally weighting CP more.
The user's framing: λ=0.7 does not mean "70% CP gradient, 30% WDL gradient"; the actual split is a function of each
term's raw residual scale. This experiment adds target/prediction/residual/dL-dOutput tracking (extending the same
`CpWdlTrace` output, no new flag, no new training runs — the existing seed 7/42/123 + shuffle-102 scratch runs were
simply rerun to regenerate `.trace.json` with the new fields) to explain the scale gap directly instead of leaving
it as inference.

## Result: `wdl_target` is a coarse per-game constant; `eval_teacher` is a fine per-position signal

**Position 1, all three unshuffled seeds (identical, since only `--init-seed` differs, not the data order)**:

| field | value |
|---|---|
| `cp_target_mean` (`eval_teacher`) | `0.00 ± 0.00` |
| `wdl_target_mean` (`wdl_target`) | `600.00 ± 0.00` |
| `prediction_mean` (network output) | `-0.03` |
| `cp_residual_mean/std` | `-0.03 ± 0.00` |
| `wdl_residual_mean/std` | `-600.03 ± 0.00` |

The first sampled position's own search-based CP evaluation happens to be exactly `0` (a genuinely balanced/quiet
position) — and the freshly-initialized network's output also starts near `0`. Their agreement is *coincidental to
this position*, not structural, but the mechanism it exposes is structural: `wdl_target` is a per-**game** constant
(the game's final result, mapped to `±600`/`0`, identical for every position sampled from that game) that has no
relationship to what the network currently outputs or to the position's actual character — so `score - wdl_target`
is inescapably close to `±600` from the very first position, regardless of how reasonable the network's (still
near-random) prediction is. `eval_teacher`, by contrast, is a genuine per-**position** search evaluation, varying
continuously position to position, including landing near `0` for quiet/balanced positions early in a game.

**This holds under both orderings, not just the specific unshuffled window**: re-running seed 7 with
`--shuffle-seed 102` still shows `wdl_target` flat (`600.00 ± 0.00`) through position 16 (this shuffled window
also happens to draw several early positions from a decisive-outcome game) — but `cp_target_std` is already
`105.27` by position 16, non-zero far earlier than the unshuffled run's `0.00 ± 0.00` at the same point. The
*specific* value (`0` at position 1) is data-dependent; the *structural* asymmetry (WDL locally flat within a
game, CP varying within it) is not.

**As the window widens, both distributions gain real spread, but WDL's residual ends up larger, not smaller**:

| position | seed | `cp_target` mean±std | `wdl_target` mean±std | `cp_residual_std` | `wdl_residual_std` |
|---|---|---|---|---|---|
| 256 | 7 (unshuffled) | 5.86 ± 394.24 | 360.94 ± 436.29 | 394.59 | 437.75 |
| 256 | 42 (unshuffled) | 5.86 ± 394.24 | 360.94 ± 436.29 | 394.72 | 438.50 |
| 256 | 123 (unshuffled) | 5.86 ± 394.24 | 360.94 ± 436.29 | 394.69 | 438.31 |
| 256 | 7 (shuffle 102) | -0.23 ± 406.23 | 131.25 ± 585.47 | 406.18 | **585.53** |

(`cp_target`/`wdl_target` at position 256 are identical across the three unshuffled seeds for the same reason
position 1 was — same data order, only initialization differs, so the same 256 positions' *targets* are visited
regardless of seed; only `prediction`/`score` and the gradients derived from it vary by seed.) Once the window
spans enough games to sample a real mix of outcomes, `wdl_target`'s own spread (bimodal-ish, clustered near
`±600`/`0`) stays large-magnitude, while `cp_target`'s spread — though it grows — reflects the real, sometimes
tightly-clustered distribution of search evaluations. `wdl_residual_std` ends up *larger* than `cp_residual_std`
in every case checked, including the shuffled one (585.53 vs. 406.18) — consistent with the previously-established
finding that WDL's gradient-magnitude dominance survives shuffling.

**Prediction stays small throughout this window** (network output mean/std grows from `~0` to single digits by
position 256, e.g. `3.85 ± 2.76` for seed 7) — far smaller than either target's scale. This means the residual
asymmetry in this window is driven almost entirely by the *targets'* own relative scales, not by the network's
prediction drifting toward one target or the other.

## Interpretation

**λ=0.7 is not the main cause of the gradient gap.** It sets the *weight* on the CP term in the convex combination
`λ(x−a)² + (1−λ)(x−b)²` (per the module's own documented equivalence — see `cp_wdl_gradient_decomposition.md`), but
the *actual* gradient contribution from each term is `λ·2(x−a)` vs. `(1−λ)·2(x−b)` — proportional to each term's own
residual, not just its weight. The current `wdl_target` encodes the game's result as a fixed value near `±600`,
applied identically to every position within that game; `eval_teacher` (the CP target) is a per-position
teacher-search evaluation. This difference in target structure and scale is what makes WDL's residual and gradient
dominate CP's, especially early in training — regardless of λ.

**Not yet concluding "the WDL target is bad."** The game result is a genuinely useful long-horizon signal for
position value — the finding here is narrower: mixing it directly into the same output, at the same numeric range,
as a per-position CP evaluation is what's unstable, not the WDL signal itself. Down-weighting λ further would
shrink WDL's contribution proportionally without addressing *why* its raw residual is so much larger to begin
with; the more targeted next lever is the target-scale mismatch in how the two are combined, not the blend weight.

**Explicitly not addressed here, per the user's own instruction**: the fully-dead L2 neurons (`docs/experiments/
epoch1_batch_trace.md`) already show exactly zero gradient from *both* CP and WDL signals — this target-scale
finding explains the *relative* size of the two signals among neurons that do receive gradient, it says nothing
about why the pinned neurons receive none from either. That remains a separate, still-open mechanism; not
conflated with this one.

## Status

- No new flag — this extends `--cp-wdl-grad-trace`'s existing output (`CpWdlTrace` gained 14 new fields:
  `cp_target_mean/std`, `wdl_target_mean/std`, `prediction_mean/std`, `cp_residual_mean/std`,
  `wdl_residual_mean/std`, `cp_d_output_mean/std`, `wdl_d_output_mean/std`).
- Reused the existing weighted-sum-identity and state-non-alteration tests (still pass unchanged — purely additive
  reads) plus one new light assertion (the new fields are populated and match the test's own constant
  `eval_teacher`/`wdl_target` inputs exactly).
- **Diagnosis, not a fix**: narrows the next lever toward the target-scale mismatch specifically — candidates now
  include rescaling `wdl_target` to a smaller native range (rather than `±600`, matching CP's more typical
  early-game magnitude), or a schedule that ramps WDL's effective weight in from a small value rather than a fixed
  λ from epoch 1. Neither implemented or tested here.
- Full per-position trace data (regenerated with the new fields): `cp_wdl_exp/` in scratch (3 unshuffled seeds +
  1 shuffled cross-check, same 4 runs used throughout this line).
