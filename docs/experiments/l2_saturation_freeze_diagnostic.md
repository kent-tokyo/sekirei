# Freeze diagnostic (task #91): FT and L2 movement are each individually *necessary* for the saturation collapse — freezing either alone fully blocks it; output-weight movement is not implicated

## Background

`docs/experiments/l2_saturation_mechanism_p0.md` (P0a/P0b) found that epoch-1's L2 saturation collapse is driven
by *correlated* FT-output and L2-weight movement (36/36 saturating neurons across 3 seeds show every decomposition
term same-signed as the total movement), but that decomposition is correlational: it can't say what happens if
only one of the two moved, since a frozen-weights Δz decomposition can't simulate a real re-run with one layer's
optimizer step disabled. This is exactly what a causal freeze diagnostic resolves — 4 arms (Control, freeze FT,
freeze L2, freeze Out), each layer's own parameter update skipped for positions 1–320 of epoch 1 while gradient
still flows through it normally (not stop-gradient), then released to run normally through position 512.

## Method

**New `--diagnostic-freeze-layer <ft|l2|out>` / `--diagnostic-freeze-until-position <n>`** (`Trainer::
diagnostic_freeze_layer: Option<FreezeLayer>`, default `None` — byte-identical to the flags not existing). While
`l2_sample_count <= diagnostic_freeze_until_position` this epoch, the named layer's own Adam update (params *and*
`m`/`v` moments) is skipped entirely; every other layer's Adam step proceeds unmodified. This is **not**
stop-gradient: the ordinary backward pass computes every layer's gradient through this layer's *current* (frozen)
weight values before the freeze gate is checked, so upstream/downstream layers keep receiving a real gradient
signal through it — freezing only discards that one layer's own parameter update. Multiple simultaneous frozen
layers aren't supported (single `Option`, not a set) — not needed to isolate one layer at a time.

**Verification, all required before trusting the results**:
- 5 new unit tests, 102 total passing, `fmt`/`clippy` clean: flag-omitted byte-identical to no freeze; each of the
  3 arms proven to (a) leave the frozen layer's params exactly unchanged and (b) leave the *other* two layers
  updating normally (proving gradient wasn't cut); freeze correctly releases and resumes updating once
  `l2_sample_count` passes the bound.
- **Independent real-run cross-check** using the already-verified `l2_delta_z_probe` (P0b's tool) against actual
  `--trace-weights` checkpoints from a live L2-freeze run: between two checkpoints *inside* the freeze window
  (position 32→320), `term2` (L2-weight movement) and `term3` (the cross term) are **exactly 0.0000** while
  `term1` (FT movement) is large (43,722) — proving both that L2 truly didn't move and that FT still received and
  applied a real gradient through the frozen L2 weights. Between checkpoints *crossing* the release boundary
  (320→512), `term2` jumps to 454,149 — proving the freeze correctly releases.
- Metadata (`diagnostic_freeze_layer`/`diagnostic_freeze_until_position`) confirmed recorded correctly in
  `.meta.json` for every arm.

**Experiment**: 4 arms × 3 seeds (42, 7, 123), `--epochs 1`, fixed `data/gateA_csa_subset`, `--split-seed 42`
(`train_games=293 valid_games=44` identical every run), `--wdl-lambda 0.7`, one shared teacher cache
(`cache_miss=0`, 11,183 hits, identical across all 12 runs — same teacher-search results everywhere), `--sample-
grad-trace 512` and `--trace-positions 0,32,64,96,128,160,192,224,256,288,320,384,512 --trace-weights`.
`--diagnostic-freeze-until-position 320` on every non-control arm.

**A shell scripting pitfall worth recording**: an initial batch built the per-arm CLI flags as a single quoted
string (`layer_flag="--diagnostic-freeze-layer ft"`) passed unquoted to the trainer invocation, relying on word
splitting to separate it into two argv tokens. This works in `bash` but **not** in this environment's default
shell (`zsh`, which does not word-split unquoted `$var` by default) — the whole string became one malformed argv
token, silently failed to match any parsing arm, and every "frozen" run silently trained as an unmodified control
run. Caught by checking every run's own `.meta.json` `diagnostic_freeze_layer` field before trusting any analysis
output, per this investigation's standing discipline — not caught by the code being wrong (it wasn't), by the
orchestration around it being wrong. Fixed by building arguments as a shell array (`layer_args=("$@")`,
`"${layer_args[@]}"`), which passes tokens unambiguously regardless of shell.

## Results

Zero-gradient reason classification (P0a's exact method, `dL/dz_L2 = dL/dOutput × output_weight ×
ClippedReLU'(z_L2)`), banded around the freeze boundary. `residual_zero`/`output_path_zero` are `0.0%` in every
arm, every seed, every band (matching P0a) — omitted from the table below; only `linear` and `clamped_high`
(saturation) are shown, since `clamped_low` (dead) is discussed separately.

| arm | positions 257–320 (frozen) `clamped_high` | positions 385–512 (post-release) `clamped_high` |
|---|---|---|
| control (seed 42/7/123) | 43.8% / 31.2% / 37.5% | 43.2% / 30.8% / 37.1% |
| freeze FT | **0.0% / 0.0% / 0.0%** | 46.9% / 31.2% / 40.6% |
| freeze L2 | **0.0% / 0.0% / 0.0%** | 52.3% / 46.1% / 49.2% |
| freeze Out | 42.7% / 29.5% / 36.6% | 43.1% / 31.0% / 36.9% |

**Freeze FT and freeze L2 both hold `clamped_high` at exactly 0.0% through the entire 320-position freeze window,
in all 3 seeds — not reduced, exactly zero.** Freeze Out is statistically indistinguishable from control at every
band, every seed — freezing the output layer's own weights has no protective effect at all.

**This is a compounding (multiplicative), not additive, mechanism.** `L2`'s pre-activation is `z = FT_output ×
W_L2 + b_L2` — pushing a neuron past the `z ≥ 127` wall requires the *product* to grow, not either factor alone.
If saturation were merely three reinforcing-but-independent additive pushes (as P0b's correlational read alone
could suggest), removing one of two reinforcing terms should still allow some accumulation by position 320 under
the other two. It doesn't — zero saturation forms with either factor pinned. **FT movement and L2-weight movement
are each individually necessary for the collapse; neither alone is sufficient (the still-moving partner alone
produces none); output-weight movement is neither.**

**Two confounds, disclosed rather than smoothed over — neither changes the conclusion above:**

- **`clamped_low` (true death) differs by arm — freeze-FT is the clean arm, freeze-L2 is not.** Freeze-FT's dead
  fraction tracks control closely in every seed (e.g. seed 42: control 56.2%, freeze-FT 50.3–54.2%) — its elevated
  `linear` fraction during freeze is purely saturation-suppression. Freeze-L2's dead fraction runs meaningfully
  *below* control in every seed (e.g. seed 42: control 56.2%, freeze-L2 39.2–46.5%) — pinning L2's weights at
  their (still-early) init values changes which neurons land in `z ≤ 0` in the first place, so part of freeze-L2's
  elevated `linear` reading is a different dead-neuron population, not purely suppressed saturation. The
  `clamped_high = 0.0%` result itself is unambiguous either way (a neuron can't be simultaneously dead and
  saturated), so the core necessity claim holds for both arms — freeze-FT is simply the cleaner demonstration.
- **Post-release relapse magnitude is confounded by pent-up FT drift, not a clean apples-to-control comparison.**
  While L2 is frozen, FT keeps moving unconstrained; once L2 releases, its weights immediately combine with an FT
  output that already drifted further than it would have under normal joint training. This is the direct
  explanation for why freeze-L2's post-release `clamped_high` (46–52%) *overshoots* control's (31–44%) rather than
  merely matching it, and why relapse reads as fast. Freeze-FT's post-release levels (31–47%) sit closer to
  control's own range, consistent with less one-sided drift accumulating during its freeze. **Relapse existing at
  all — in both arms, in all 3 seeds — is the clean, seed-consistent claim; relapse *speed* and *overshoot
  magnitude* are reported as observations, not as calibrated rates comparable to control.**

**Direct line back to the original motivating finding**: `sample_grad_correlation_trace.md` found L2's gradient
collapses to all-zero for ~100% of positions by ~1/4 into epoch 1. The zero-gradient rate here is exactly
`100% − linear%`. During the freeze window, control sits at ~100% zero (gradient path fully closed) while
freeze-FT sits at ~53–65% zero (path substantially still open) — freezing either layer alone keeps L2's gradient
path from closing; it slams shut only once both layers are free to move together.

**`valid_output_std` (secondary, per the pre-registered framework)**: noisy and not load-bearing — several arms
read exactly `0.0` (the known output-constant collapse pathology this whole line has documented elsewhere), and
the metric is computed once at epoch end (position ~9723), far past the 512-position window this diagnostic
actually probes. Reported for completeness, no conclusion drawn from it: control `{19.4, 0.0, 0.0}`, freeze-FT
`{2.6, 26.5, 0.0}`, freeze-L2 `{25.8, 7.7, 0.0}`, freeze-Out `{~0, 0.0, ~0}` (seeds 42/7/123 respectively).

## Conclusion

**Direction 2 (the structural question) is closed with a decisive, 3-seed-consistent, causally-verified answer.**
The epoch-1 L2 saturation collapse requires *both* FT-output movement and L2-weight movement acting together —
freezing either one alone (not stop-gradient; gradient still measurably reaches and updates the other layers)
fully suppresses new saturation for as long as the freeze holds, in every seed, with zero exceptions. Output-layer
weight movement is not implicated at all — freeze-Out is indistinguishable from control. This retires the
output-scale-runaway→L2 causal thread specifically for the *weight-movement* channel (P0a's `output_path_zero =
0.0%` already retired it for the *gradient-computation* channel) — output backprop *structure* was never in
question (gradient always flowed correctly in every arm; that's what "not stop-gradient" verified), only whether
the output layer's own weight movement was driving the correlated push, and it isn't.

**This is suppression, not elimination.** Both single-layer freezes show collapse resuming once released, within
roughly one to two trace intervals (~64–128 positions). Freezing one layer removes one necessary factor from the
product temporarily; it doesn't change the underlying dynamics that make the product grow once both factors are
free again. Whether a longer freeze window, a joint FT+L2 freeze, or a fundamentally different intervention (e.g.
directly bounding the product/pre-activation scale) would prevent the eventual collapse rather than just delaying
it is an open question this diagnostic wasn't designed to answer — the pre-registered decision table's "try a
combined-layer freeze" branch was conditioned on *no single-layer freeze working*, and here both did, so that
branch doesn't trigger as an automatic next step.

## Status

- `FreezeLayer` enum, `Trainer::diagnostic_freeze_layer`/`diagnostic_freeze_until_position`, the 2 CLI flags, and
  5 new unit tests are implemented and verified (102 tests, `fmt`/`clippy` clean) but **not yet committed**.
- 12-run experiment (4 arms × 3 seeds) complete; raw data and analysis script in scratch (`freeze_exp/`:
  `classify_freeze_arm.py`, `*.epoch1.sample_grad.jsonl`, `*.epoch1.pos*.bin` trace-weight snapshots).
- Direction 1 (P1, the `--shuffle-seed` order-sensitivity sweep, deferred behind Direction 2 per the original
  sequencing decision) is next, pending explicit go-ahead — not started.
