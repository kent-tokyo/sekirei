# King-relative NNUE (B-small) — scale-contract static audit

Status: **static, read-only.** No `cargo build`/`test`/`check`/`clippy`, no
training, no checkpoint re-inference, no engine runs, no matches were
performed to produce this document. Every claim cites the specific source
line it's derived from (`main`@`c1719fe`, the commit all 6 Phase 2 training
runs were built from), or a scalar field already present in an existing
`.epoch20.meta.json` sidecar. Written as a follow-up to
`docs/experiments/king_relative_b_small_phase3_diagnostic.md`, which found
Phase 3's `valid_cp_mse` PASS (3/3 seeds) sits alongside a `valid_wdl_loss`/
`valid_calibration_error` regression (0/3 seeds) that isn't explained by
output-clamping (refuted there). This audit investigates whether a
scale/unit mismatch between architectures explains the gap instead.

## 1. Raw output → CP conversion (multiplier/units)

**Confirmed, identical for A and B-small.** Both the runtime engine
(`nnue.rs:659`, `(out / 64.0) as i32`) and the training-time forward pass
(`trainer.rs:1819`/`1995`, `output / 64.0`) divide the final layer's
accumulated output by the same fixed constant, `64.0`. Neither function has
a `#[cfg(feature = "king_relative_b_small")]` branch anywhere near this
divisor — confirmed by `grep -n "cfg(feature" crates/sekirei-core/src/nnue.rs
crates/sekirei-train/src/trainer.rs`, whose 6 total matches are all in
`feature_index`'s body, `king_zone`-adjacent code, and the weight-file
magic string (`nnue.rs:365`, `493`, `73`, `81`), never in `evaluate`/
`forward`. The `64.0` itself is `FT_SCALE` (`nnue.rs:631`), the same
constant used to dequantize the i16 FT accumulator back to `f32` — a
property of the *quantization* scheme, not the network's logical
architecture, and untouched by `INPUT`'s size.

## 2. CP target scale and normalization during training

**Confirmed, identical for A and B-small.** `eval_teacher =
clamp(search_score_cp, ±600)` (`trainer.rs:9`, module doc). No architecture
branch in `position_teacher_components` (the function that produces
`eval_teacher`) or its callers. Empirically confirmed identical across all
6 runs beyond just reading the code: every one of the 6
`.epoch20.meta.json` files has **byte-identical** `target_mean` (67.194...)
and `target_std` (393.008...) — this field is a training-set diagnostic of
the (possibly WDL-blended) teacher's own distribution
(`main.rs:965-969`, `diagnostics::mean_std` over `trainer.target_sum`/
`target_sum_sq`), and its exact equality across every seed/architecture
combination confirms the target side of the loss is 100% dataset/config-
derived, with zero architecture dependency — any difference in `score`'s
own distribution (§7) originates entirely from what each network learned
to output, not from a different target.

## 3. WDL target construction

**Confirmed, identical for A and B-small.** `wdl_target_cp`
(`trainer.rs:62-82`): `(wdl − 0.5) × scale`, where `wdl ∈ {0.0, 0.5, 1.0}`
from `GameResult` and `scale` is the `--wdl-target-scale` CLI value
(default `1200.0`, mapping to `∓600`). No architecture branch. Empirically
confirmed identical across all 6 runs: `wdl_target_scale` is `1200.0` in
every `.epoch20.meta.json`, including both architectures — `run_king_relative_phase2.sh`
never passes `--wdl-target-scale`, so every run uses the same compiled-in
CLI default regardless of architecture.

## 4. CP→probability conversion inside WDL loss

**Confirmed, and the premise needs correcting: `valid_wdl_loss` itself
does not perform a CP→probability conversion at all.** `eval_game`
(`trainer.rs:1758-1761`) computes `wdl_err = score − wdl_target` and
accumulates `wdl_err²` — both `score` and `wdl_target` stay in raw
∓600-scale CP units throughout; there is no division by `wdl_target_scale`
and no clamping anywhere in the `valid_wdl_loss` computation itself. The
`score / wdl_target_scale + 0.5` mapping only appears in the calibration
bucket computation two lines later (§5). `valid_wdl_loss` is a raw squared
CP-scale error against the game-outcome-derived target, structurally
identical in shape to `valid_cp_mse` (`trainer.rs:1755-1756`, `cp_err =
score − eval_teacher`) — same units, same code shape, different target.

## 5. Whether calibration's conversion formula matches (and what it implies)

**Confirmed.** `predicted_prob` and `actual_prob`
(`trainer.rs:1765-1766`) both apply the exact same linear map,
`(x / wdl_target_scale) + 0.5` clamped to `[0, 1]` — one applied to
`score`, the other to `wdl_target`. Because both sides of the calibration
comparison go through the *identical* transform, `valid_calibration_error`
is mathematically a monotonic-ish re-expression of "how far `score` sits
from `wdl_target`" (bucketed mean-absolute-difference of the mapped
values, per `CALIBRATION_BUCKETS=10` deciles of `predicted_prob`) — the
same underlying comparison `valid_wdl_loss` already makes (§4), just
aggregated differently (bucketed mean-abs-diff instead of raw MSE, and
with the ±600 range compressed into [0,1] before differencing). **This is
why `valid_wdl_loss` and `valid_calibration_error` moved in the same
direction in all 3 seeds** (Phase 3 diagnostic doc, §3/§5 there) — they
are not two independent lines of evidence for a calibration problem, they
are the same `score`-vs-`wdl_target` gap read two ways. A genuinely
independent calibration check would need a metric that doesn't reuse the
same linear map for both sides (out of scope to build here).

## 6. Whether using the same conversion for A and B-small is valid

**Confirmed as fact (same code path, no branch, §1-§5); the
appropriateness judgment is a separate, open question addressed in §9.**
Nothing in the loss/calibration code reads `cfg!(feature =
"king_relative_b_small")` or any other architecture signal — the "same
conversion" is not a design choice made per-architecture, it's the absence
of any per-architecture path at all.

## 7. Why `valid_cp_mse` can improve even though `output_std` grows ~1.8×

**Plausible, not confirmed** — mathematically sound in principle, directionally
consistent with an existing (but not exactly matching) saved statistic,
not verifiable exactly without new inference. The exact identity:

```
MSE = E[(score − teacher)²]
    = Var(score) + Var(teacher) − 2·Cov(score, teacher) + (E[score] − E[teacher])²
```

A `Var(score)` increase (wider `output_std`) can still produce a *lower*
MSE if `Cov(score, teacher)` grows enough to outweigh it — which happens
exactly when `score` becomes more strongly correlated with the true
target, not just noisier. The one correlation figure saved
(`pred_eval_correlation`, `main.rs:970-977`) is **higher for B-small in
every seed** (0.636–0.659 vs. A's 0.540–0.564, per the Phase 3 diagnostic
doc's table) — directionally consistent with this explanation. **The
caveat that keeps this at "plausible" rather than "confirmed"**:
`pred_eval_correlation` is a **training-set** statistic, accumulated
inside `train_position` via `trainer.output_sum`/`eval_teacher_sum`/
`pred_eval_prod_sum` (`main.rs:970-977`) — not the validation set that
`valid_cp_mse`/`valid_output_std` are measured on. No validation-set
covariance or correlation figure is saved anywhere in the sidecar. The
identity above is exact; whether it's *the* mechanism behind the specific
observed validation-set `cp_mse` improvement can't be numerically confirmed
from what's saved, only judged directionally plausible from the adjacent
training-set number.

## 8. Whether affine recalibration (`a·score + b`) alone could explain the WDL/calibration gap

**Plausible, explicitly unconfirmed and unconfirmable from what's saved.**
Structurally, both `valid_wdl_loss` and `valid_calibration_error` are
built entirely from linear comparisons of `score` against `wdl_target`
(§4-§5) — a systematic linear scale/offset mismatch between B-small's
`score` and the scale `wdl_target`/the loss function implicitly assumes
(the same `wdl_target_scale=1200` used for every architecture, §3) is a
structurally available failure mode, and B-small's `output_mean`/
`output_std` both run ~1.4-1.8× A's in every seed (Phase 3 diagnostic
doc §3) — consistent with, but not proof of, exactly this. Per the
standing instruction for this pass: **no per-sample prediction file was
ever saved for any of the 6 runs** (confirmed in the Phase 3 diagnostic
doc, §2 there), so there is nothing to retroactively fit an affine
correction against, even as a "try this later, cheaply" note — that note
would only be honest if raw predictions already existed. They don't. This
hypothesis stays **frozen, unconfirmed**, not "confirmed-but-deferred."

## 9. Whether the lack of a per-architecture scale parameter is a design gap

**Confirmed as an absence of fact (§1-§6); "gap" is a judgment call, not a
fact this audit can settle.** There genuinely is no per-architecture output
scale, temperature, or affine-calibration parameter anywhere in
`nnue.rs`/`trainer.rs` — `FT_SCALE=64.0`, the `ClippedReLU(0, 127)` clamps
at every layer, and the final `/64.0` divisor are all compile-time
constants shared unconditionally by every build. Whether that absence is a
"gap" depends on a premise this audit cannot verify without new data:
if B-small's *raw* representational content (per §7's higher
`pred_eval_correlation`) is genuinely better but arrives at a different
natural scale purely as a training-dynamics artifact (nothing in the
architecture *requires* A and B to converge to the same output scale, and
nothing observed here rules that reading out), then a per-architecture (or
even per-checkpoint) affine recalibration layer would be a reasonable,
structurally cheap fix. If instead the wider scale reflects a genuine
degradation the linear WDL mapping is correctly penalizing, adding a
scale-hiding parameter would launder that regression rather than fix it.
Distinguishing these two readings needs the same per-sample data or
re-inference this pass was explicitly scoped to avoid.

## Summary classification

| # | Question | Verdict |
|---|---|---|
| 1 | Raw output→CP multiplier/units | **Confirmed** identical (`/64.0`, no arch branch) |
| 2 | CP target scale/normalization | **Confirmed** identical (clamp ±600, byte-identical `target_mean`/`std`) |
| 3 | WDL target construction | **Confirmed** identical (`(wdl−0.5)×1200`, no arch branch) |
| 4 | CP→prob conversion in WDL loss | **Confirmed** — premise was wrong; `valid_wdl_loss` never converts to probability at all |
| 5 | Match vs. calibration's conversion | **Confirmed** identical mapping on both sides; explains why WDL loss and calibration move together (not independent signals) |
| 6 | Same conversion valid for A/B-small | **Confirmed** as fact (no branch exists); appropriateness deferred to §9 |
| 7 | Why cp_mse can improve despite wider output_std | **Plausible** (MSE identity is exact; higher train-set `pred_eval_correlation` for B is directionally consistent, but validation-set covariance isn't saved to confirm numerically) |
| 8 | Affine recalibration explains WDL/calibration gap | **Plausible**, frozen unconfirmed — no per-sample data exists to test against, this round or later without new inference |
| 9 | Missing per-arch scale parameter = design gap | **Confirmed** absence of fact; "gap" judgment explicitly not resolved here |

**Nothing in this audit changes Phase 3's frozen status.** Restated from
`king_relative_b_small_phase3_diagnostic.md`: `valid_cp_mse` 3/3 improved,
`valid_wdl_loss`/`valid_calibration_error` 0/3 improved (and are now shown
to be largely the same underlying signal, not two independent
regressions), the clamp-artifact explanation is refuted, and this audit's
two open hypotheses (§7's covariance story, §8's affine-mismatch story)
both require data this pass deliberately did not collect. **Phase 5
remains on hold**, per instruction, pending a decision on the still-open
questions rather than proceeding automatically from this document's
completion.
