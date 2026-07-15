# Gradient clipping (global and output-only): trims tails, doesn't fix generalization or runaway

## Background

Follow-up to the gradient/update-norm diagnostics added alongside this work
(`d693028`): the whole-network gradient-norm distribution is heavy-tailed
and strongly non-stationary within a single run (B_seed42's own p95 grew
9140→20659→54579 across 3 epochs of a 3-epoch slice). Gradient clipping was
tested as a possible fix for two distinct, separately-tracked failure
modes this recipe shows: **(A)** epoch-2+ output-scale runaway
(`output_weight_norm`/`valid_output_std` growing without bound), and
**(B)** an epoch-1 `valid_output_std≈0` collapse in some seeds. Two
single-variable experiments were run in sequence, both on the B recipe
(`--wdl-lambda 0.7 --lr-schedule cosine --lr-schedule-epochs 20 --epochs 3`,
seeds 42/7/123), sharing one teacher-search cache.

## Experiment 1: global-norm clipping (`--grad-clip-norm`, commit `de23164`)

Scales every layer's gradient by the same factor when the whole-network
(FT+L2+out) gradient norm exceeds a threshold. Threshold **9000**, chosen
as B_seed42's own epoch-1 global-norm p95.

| | seed42 | seed7 | seed123 |
|---|---|---|---|
| epoch2→3 `output_weight_norm` growth, control | 1.779 | 1.798 | 1.882 |
| epoch2→3 `output_weight_norm` growth, candidate | 1.872 | 1.888 | 1.986 |
| epoch3 `valid_cp_mse`, control → candidate | 158859→163407 | 166377→162450 | 164991→160484 |

**FT gradient tail is measurably trimmed** — confirmed via a confound-free
discriminator (seed7's epoch1 was byte-identical control-vs-candidate, so
its epoch2 divergence starts from identical weights): FT gradient-norm mean
dropped 565→352 (38%, at 2.2% clip rate); seed123's clean-enough epoch2
(749→1389, 46% at 10.1% clip rate) matches. This is a real, non-trivial
effect, not noise.

**Against the actual goals, this doesn't help**:
- Can't touch the epoch-1 collapse (clip rate ~0-2% at epoch1, the
  threshold barely engages there).
- **`output_weight_norm` growth is slightly *faster* under clipping in all
  3 seeds** (delta +0.09 to +0.10) — the opposite of the intended effect.
- `valid_cp_mse`: 2 improve (seed7, seed123), 1 worsens (seed42, the same
  seed that transiently collapsed) — a *reordering*, not a variance
  reduction (full detail in `tasks/lessons.md`'s 2026-07-15 entry), so not
  read as a stabilization effect either.
- The threshold was tuned to seed42's own distribution, and seed42 is the
  seed clipping most visibly disturbed (a transient epoch-1 collapse in the
  candidate arm not present in control) — a mild circularity.

**Working hypothesis at the time** (before Experiment 2 existed): the
output layer's raw gradient scale (~4000-4400) dominates the global norm,
so a global clip calibrated to rein in the output layer ends up
disproportionately shrinking FT/L2's much-smaller-baseline gradients on the
same clipped positions — "collateral damage" to layers that weren't
causing the spike. This predicted that clipping *only* the output layer's
gradient, leaving FT/L2 completely untouched, should recover the intended
effect (runaway suppression) without the FT/L2 downside. Experiment 2
tests that prediction directly.

Full per-epoch table, logs, `.meta.json`: `grad_clip_exp/` in scratch.
Artifact: <https://claude.ai/code/artifact/6253fef5-bcb4-4708-b182-2c4aba3356c4>

## Experiment 2: output-only clipping (`--out-clip-norm`, commit `4d583f1`)

Independent per-layer thresholds (`--ft-clip-norm`/`--l2-clip-norm`/
`--out-clip-norm`), each layer clipped against its own gradient norm only.
`--ft-clip-norm`/`--l2-clip-norm` left unset for this run — a dedicated
unit test (`train_position_out_clip_norm_leaves_ft_and_l2_untouched`)
proves FT/L2 stay byte-identical to an unclipped run when only
`out_clip_norm` is set.

**Threshold selection**: the output layer's *own* gradient-norm
distribution, not reused from Experiment 1. Measured on a clean run:
epoch1 p95=9120/p99=9178, epoch2 p95=9552/p99=10549, epoch3 p95=7641/
p99=10945 — far flatter across epochs than the global norm's was, so one
fixed value is a reasonable fit for the whole run. Chose **9000** (≈
epoch1's own p95, same numeral as Experiment 1 by coincidence of both
being anchored to an epoch-1 p95, not because it's the same measurement).

Control = no clip. Candidate = `--out-clip-norm 9000` only.

| tag | ep | ft_clip% | l2_clip% | out_clip% | `valid_cp_mse` | `output_weight_norm` |
|---|---|---|---|---|---|---|
| seed42 control | 1 | 0.00 | 0.00 | 0.00 | 172909.7 | 6.994 |
| seed42 candidate | 1 | 0.00 | 0.00 | 1.92 | 173533.0 | 4.829 |
| seed42 control | 2 | 0.00 | 0.00 | 0.00 | 161521.1 | 15.015 |
| seed42 candidate | 2 | 0.00 | 0.00 | 2.49 | 170350.5 | 12.266 |
| seed42 control | 3 | 0.00 | 0.00 | 0.00 | **158858.8** | 26.717 |
| seed42 candidate | 3 | 0.00 | 0.00 | 2.44 | 165365.1 | 24.392 |
| seed7 control | 1 | 0.00 | 0.00 | 0.00 | 173534.0 | 4.450 |
| seed7 candidate | 1 | 0.00 | 0.00 | 0.00 | 173534.0 | 4.450 |
| seed7 control | 2 | 0.00 | 0.00 | 0.00 | 173413.3 | 7.996 |
| seed7 candidate | 2 | 0.00 | 0.00 | 0.67 | 173452.9 | 7.998 |
| seed7 control | 3 | 0.00 | 0.00 | 0.00 | 166377.4 | 14.378 |
| seed7 candidate | 3 | 0.00 | 0.00 | 7.61 | **165108.1** | 14.663 |
| seed123 control | 1 | 0.00 | 0.00 | 0.00 | 173513.8 | 4.713 |
| seed123 candidate | 1 | 0.00 | 0.00 | 0.00 | 173513.8 | 4.713 |
| seed123 control | 2 | 0.00 | 0.00 | 0.00 | 171235.7 | 10.403 |
| seed123 candidate | 2 | 0.00 | 0.00 | 3.39 | 171228.1 | 10.430 |
| seed123 control | 3 | 0.00 | 0.00 | 0.00 | 164990.5 | 19.580 |
| seed123 candidate | 3 | 0.00 | 0.00 | 10.37 | 165073.0 | 19.942 |

**Isolation worked exactly as designed**: `ft_clip_trigger_rate` and
`l2_clip_trigger_rate` are 0.0% in every one of the 18 epoch-checkpoints
across both arms — FT/L2 gradients are provably untouched by this
intervention. `out_clip_trigger_rate` stays low (0-10.4%) and, like
Experiment 1's global rate, rises across epochs in seed7/123 (0%→
0.7%→7.6% and 0%→3.4%→10.4%); seed42 is flatter (~2% throughout), the same
kind of per-seed inconsistency Experiment 1 showed.

**epoch2→3 `output_weight_norm` growth ratio** (the runaway-suppression
metric):

| seed | control | candidate | delta |
|---|---|---|---|
| 42 | 1.779 | 1.989 | +0.209 |
| 7 | 1.798 | 1.833 | +0.035 |
| 123 | 1.882 | 1.912 | +0.030 |

**Grows faster under clipping in all 3 seeds** — the same direction as
Experiment 1, this time with FT/L2 provably uninvolved.

**epoch3 `valid_cp_mse`, candidate − control**: seed42 **+6506.3** (worse,
the largest single-run regression seen in either experiment), seed7
**−1269.3** (better), seed123 **+82.5** (flat). One win, one loss, one wash
— not 3/3, not even a clean 2/1.

**A new n=1 observation, not over-read**: seed42's candidate arm collapses
at epoch1 (`valid_output_std=0.0`, `valid_output_range=0.0`) while its own
control arm does not (`std=19.4`, `range=63.7`) — the same
transient-epoch-1-collapse-in-the-clipped-arm pattern Experiment 1 showed
for this seed. Seed7 and seed123 collapse identically in *both* arms
(byte-identical epoch1, 0% out_clip trigger), so this isn't "output
clipping causes epoch-1 collapse" in general — it's specific to seed42,
recurring across two different clipping mechanisms. Worth flagging as a
seed42-specific fragility, not generalized further with n=1.

## Applying the pre-registered decision framework

Four buckets were fixed before this run:
1. 3/3 seeds improve `cp_mse` AND runaway reduced AND FT/L2 not worsened → promote to quick-gate candidacy.
2. ~2 wins/1 loss → unstable, do not adopt.
3. MSE flat but output-runaway specifically improves → safety-measure candidate only, not a strength claim.
4. No improvement at all → end the clipping investigation line.

Runaway suppression (`output_weight_norm` growth) is worse, not better, in
**all 3 seeds** — this fails the AND-gate in bucket 1 outright, and also
fails bucket 3's precondition (runaway must improve). `cp_mse` is 1
win/1 loss/1 wash, which doesn't even reach bucket 2's "~2 wins/1 loss."
**Result: bucket 4 — no improvement on the metric the whole clipping line
was meant to fix, and no improvement to fall back on as a safety measure.**

## Conclusion

**A correction to the mechanism this investigation started with.**
Experiment 1's own working hypothesis (see above) was that global
clipping's failure was caused by collateral suppression of FT/L2's much
smaller gradients when the output layer's spikes force a global scale-down.
That hypothesis predicts output-only clipping — which leaves FT/L2 exactly
as they would be without any clipping — should recover the intended
effect. **It doesn't.** Output-only clipping shows the identical failure
pattern as global clipping on both the metrics that matter: no consistent
`valid_cp_mse` improvement, and `output_weight_norm` growth is faster under
clipping in all 3 seeds, exactly as it was under global clipping. Since
FT/L2 are provably byte-identical to an unclipped run throughout (0.0%
trigger rate everywhere), this failure cannot be attributed to FT/L2
collateral damage — there isn't any in this experiment.

**Revised conclusion**: neither global nor output-only gradient-norm
clipping improves generalization or suppresses output-scale runaway on
this recipe. Global clipping measurably trims FT's gradient tail as a
side effect, but that tail-trimming doesn't translate into better
`valid_cp_mse` or a slower-growing output layer either way. Tail-clipping
the output gradient — whether as part of a global clip or on its own —
simply isn't acting on whatever actually drives output-scale growth or the
epoch-1 collapse in this recipe. The mechanism remains unexplained; ruling
out FT/L2 collateral damage narrows the search but doesn't answer it.

## Status

- `--grad-clip-norm`/`--ft-clip-norm`/`--l2-clip-norm`/`--out-clip-norm`
  all stay in the codebase as optional diagnostic/safety flags. Default
  remains disabled/unset in every case; none is auto-enabled.
- **Not promoted** as part of the training recipe, in any configuration
  tested.
- **The gradient-clipping investigation line ends here**, per the
  pre-registered bucket-4 outcome above. No further clipping variants
  (per-layer combinations, decaying thresholds, warmup-gated clipping)
  are planned unless new evidence reopens the question.
- The epoch-1 `output_std≈0` collapse (failure mode B) remains unsolved
  and is explicitly a separate investigation — clipping was never expected
  to fix it (clip rates are ~0-2% at epoch1 in both experiments, the
  threshold barely engages there) and did not. Next candidate: a
  warmup-with-vs-without matched experiment, or an
  initialization/target-scaling investigation — not yet started.

Full per-epoch table, logs, `.meta.json` for both arms of Experiment 2:
`out_clip_exp/` in scratch.
