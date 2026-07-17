# B5 counterfactual replay: CP and WDL components pull the FT collapse in opposite directions

## Background

`l2_b5_ft_unit_collapse.md` established that B5 (`144-175`) active alone drives a radial FT-representation
collapse: `cos(h_before, Δh) ≈ -0.94`, and the fraction of the `512` FT-output units sitting at exactly `0.0`
(dead) jumps from `~1%` to `43-48%` within that one 32-position window. That result used the normal blended
teacher (`λ·eval_teacher + (1-λ)·wdl_target`, `λ=0.7`). This experiment asks which component of the blend drives
the collapse, via **counterfactual replay**, not an exact decomposition: from the shared position-143 state,
replay B5's own 32 positions three ways — the normal blend, CP alone, WDL alone — each using its own effective
coefficient (`λ` or `1-λ`) rather than renormalized to `1.0`. This is exact for a *single* squared-error gradient
step (the gradient is linear in the teacher value), but not across the 32-step Adam trajectory used here — Adam's
`m`/`v` moments and `√v̂` normalization are nonlinear, so a component-only run's 32-step endpoint is not guaranteed
to equal "the blended run's gradient minus the other component." Findings below are read as counterfactual
outcomes, not as a clean additive split of the blended result.

**Mechanism** (`ReplayComponent`, `Trainer::replay_override`, committed `6376f2d`): within
`[diagnostic_replay_from_position, diagnostic_replay_until_position]`, `train_game`'s normal
`(teacher, weight)` pair is replaced by `(eval_teacher, weight·λ)` for CP-only or `(wdl_target, weight·(1-λ))`
for WDL-only. Default (`diagnostic_replay_component: None`) leaves `train_game` byte-identical to today's
behavior. `eval_game` (validation) is untouched — the override only applies to the training path.

**Design**: 3 arms (Blended / CP-only / WDL-only) × 3 seeds (42, 7, 123), all starting from B5's shared
position-143 state (`16-143` frozen identically in every arm), replaying `144-175` under each teacher condition,
then continuing normally. Blended replay uses `λ=0.7` scaled by `λ` exactly as normal training already does —
included as a same-machinery consistency check, not a new condition. Fourth arm: **Frozen**, the pre-existing
`all_frozen` checkpoint from `l2_saturation_ft_freeze_block_screen_stage_b.md` (`16-271` fully frozen, no
reactivation), reused as the zero-movement negative control — see the correction below for why a *new* frozen
run was not used.

**Metrics**: `l2_alignment_formation_probe` (radial/orthogonal decomposition, dead FT-unit count, pooled over the
261-position probe set) and `l2_threshold_crossing_probe` (`‖Δθ_FT‖`, per-neuron `cos(h,w_L2)`) at position 143
and 176; `l2_saturation_probe` (L2-side dead/linear/saturated fractions) at position 176.

## Correction: the first Frozen arm did not share position-143 state with the other 3 arms

The initial attempt trained a new `single_frozen_B5_fine` arm using only
`--diagnostic-freeze-from-position 144 --diagnostic-freeze-until-position 175` (FT active and free from `16-143`,
frozen only at `144-175`). The other 3 arms use `--diagnostic-freeze-from-position 16
--diagnostic-freeze-until-position 271` plus a reactivation window at `144-175` (FT frozen from the start, with
`144-175` reopened for the replay component) — a different, already-frozen-from-position-16 trajectory through
`16-143`. A `cmp -s` check confirmed the two position-143 checkpoints differ, so `single_frozen_B5_fine` is not a
valid zero baseline for this comparison — it starts from a different state.

Fix: substitute the pre-existing `all_frozen` arm (Stage B, `16-271` fully frozen, no reactivation at all) as the
Frozen baseline. Re-verified directly for this doc:

- `cmp -s` on the position-128 checkpoint: **byte-identical** between `all_frozen` and both `replay_cp_B5`/
  `replay_wdl_B5`, all 3 seeds (position 128 is the latest shared snapshot short of 143; both trajectories are
  frozen identically through this point, so this is the expected and sufficient equivalence check).
- `all_frozen`'s own dead-FT-unit count and `‖Δh‖` over its equivalent window were already established as exactly
  static in `l2_b5_ft_unit_collapse.md` (dead units unchanged position-to-position, `‖Δh‖ = 0.0000`, all 3 seeds)
  — reused here rather than re-run, since freezing FT is deterministic and re-running would only reproduce the
  same zero.

## Results

### FT-output radial collapse: Blended most destructive, CP-only partial, WDL-only reverses sign

Pooled over the 261-position probe set, position 143→176:

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| **Blended** — `cos(h,Δh)` | -0.940 | -0.941 | -0.936 |
| **Blended** — dead units @176 | 217.9 (42.6%) | 247.3 (48.3%) | 236.7 (46.2%) |
| **CP-only** — `cos(h,Δh)` | -0.525 | -0.577 | -0.525 |
| **CP-only** — dead units @176 | 139.3 (27.2%) | 158.0 (30.9%) | 157.6 (30.8%) |
| **WDL-only** — `cos(h,Δh)` | **+0.600** | **+0.538** | **+0.571** |
| **WDL-only** — dead units @176 | 52.0 (10.2%) | 58.0 (11.3%) | 58.2 (11.4%) |
| **Frozen** (`all_frozen`) — dead units | static, `‖Δh‖=0` (see above) | | |

All arms share the same `dead units @143` value (`6.81`/`6.13`/`6.30` for seeds 42/7/123 respectively — matches
`l2_b5_ft_unit_collapse.md`'s own numbers, confirming shared position-143 state). The Blended arm's own numbers
here reproduce `l2_b5_ft_unit_collapse.md`'s original `single_active_B5` results exactly (same `cos`, same dead
counts, all 3 seeds) — expected, since Blended replay with `λ=0.7` is mechanically the same computation as normal
training, and serves as a built-in consistency check on the replay machinery itself.

**WDL-only does not reproduce the collapse — it reverses its sign.** `cos(h,Δh)` flips from strongly negative
(Blended, CP-only) to positive (`+0.54` to `+0.60`): the WDL-only update grows the FT representation along its
existing direction rather than shrinking it. CP-only partially reproduces the collapse direction (`cos ≈ -0.53`
to `-0.58`, roughly 55-62% of Blended's cosine magnitude) at a smaller dead-unit fraction than Blended.

### FT parameter movement is largest for WDL-only, despite WDL-only being the least FT-destructive

`‖Δθ_FT‖` over the same `143→176` window (position-143 checkpoint → position-176 checkpoint):

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| Blended | 379.4 | 374.5 | 372.9 |
| CP-only | 364.4 | 358.9 | 358.7 |
| WDL-only | **480.3** | **478.9** | **480.0** |

WDL-only moves FT parameters the *most* of the three arms (~28-30% more than Blended) while producing the
*fewest* dead units and the only sign-reversed `cos(h,Δh)`. Raw movement magnitude is not what predicts
FT-representation damage — direction and which teacher signal drives it matter far more than how far the
parameters travel.

### L2-side dead-neuron fraction is inverted relative to the FT-side ranking

`l2_saturation_probe` at position 176, 32 L2 neurons, pooled over the 261-position probe set:

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| Blended | 21.3% | 17.3% | 16.0% |
| CP-only | 31.5% | 45.4% | 27.5% |
| WDL-only | **49.7%** | **68.7%** | **59.3%** |

FT-side dead-unit ranking is Blended > CP-only > WDL-only (most to least FT damage). L2-side dead-neuron ranking
is the reverse: WDL-only > CP-only > Blended. WDL-only leaves the *most* FT units alive but drives the *most* L2
neurons dead — meaning FT-unit survival alone does not predict L2-side health. A plausible reading is that
WDL-only's surviving FT units point in directions that don't align with the L2 rows they feed, so L2's weighted
input still collapses toward its own dead floor even though FT's own norm is intact or growing — but this is an
observation, not yet tested (would need FT-vector·L2-row dot products / `cos(h,w_L2)` per unit, not computed
here). Left open, not investigated further in this experiment.

### Observed anomaly: seed 7's `valid_output_std` is exactly zero in both component-only arms

`valid_output_std` at epoch end: CP-only seed 7 = `0.0`, WDL-only seed 7 = `0.0` (nonzero in both arms for seeds
42 and 123: `35.6`/`8.4` for CP-only, `20.7`/`12.1` for WDL-only). Seed 7 is also the seed with the deepest
collapse in every arm above (highest dead-unit fraction, highest L2-dead fraction). Consistent with a full output
collapse to a constant by the end of epoch 1 for this seed under either component-only replay, but not
diagnosed further here — noted as an observation.

## Conclusion

**Blended replay produces a deeper FT collapse than either component-only replay; whether this excess reflects
within-step interaction, threshold accumulation, or trajectory divergence remains unresolved.** What is
established:

- CP pushes FT in the direction opposite its existing representation and alone causes substantial dead-ification
  (`27-31%` of all FT units).
- WDL grows FT's norm along its existing direction (sign-reversed `cos`) but strongly increases L2-side
  dead-ification — FT-side survival does not imply L2-side health.
- Blended replay shows the deepest FT collapse of the three arms in every seed.
- Raw parameter-movement magnitude is not what predicts FT damage: WDL-only moves FT parameters the most while
  damaging FT the least.
- Whether Blended's excess destruction beyond CP-only alone is genuine within-step gradient/optimizer
  interaction, cross-step threshold accumulation, or simple 32-step trajectory divergence between the
  differently-updated arms is **not** distinguishable from this 32-step trajectory comparison alone — a naive
  dead-unit-count subtraction cannot rule out unit overlap, per-unit threshold effects, trajectory divergence
  across the 32 steps, or Adam's own nonlinearity in the gradient-to-update transform.

Not classified as "synergy confirmed." That question is deferred to a planned B5-limited one-step shadow trace
(branching CP-only/WDL-only/Blended one-step counterfactuals from an identical shared pre-update state at each
of B5's 32 positions, each evaluated once and discarded without mutating real training state), which eliminates
the trajectory-divergence confound this experiment cannot rule out.

## Scope

Radial/orthogonal decomposition, dead-unit counts, and L2 dead/linear/saturated fractions are pooled over the
fixed 261-position probe set, not live training data. `‖Δθ_FT‖` is the raw FT-parameter-slice movement norm
between the position-143 and position-176 checkpoints, not a per-step quantity. The FT-vs-L2 inversion is
reported as an open observation, not explained here.

## Status

- `ReplayComponent` / `Trainer::replay_override` / `--diagnostic-replay-component`,
  `--diagnostic-replay-from-position`, `--diagnostic-replay-until-position`: committed (`6376f2d`), 4 new unit
  tests (unset-is-byte-identical, CP scaling, WDL scaling, out-of-window no-op), 116/116 `sekirei-train` tests
  passing, `fmt`/`clippy` clean.
- 9 runs (3 arms × 3 seeds) complete, self-verified against requested `.meta.json` config, Frozen-arm
  substitution re-verified (byte-identical position-128 checkpoints, both components, all 3 seeds).
- Follow-up: the within-step-interaction-vs-trajectory-divergence question left open above is resolved in
  `l2_b5_shadow_trace.md` — a one-step shadow trace (identical shared anchor per position, eliminating trajectory
  divergence by construction) found the within-step Adam effect is *sub-additive* (protective, not destructive),
  so the deep 32-step Blended collapse is cross-step accumulation, not within-step interaction. The FT-vs-L2
  inversion remains unresolved (per-unit `cos(h,w_L2)` needs the *trained* 32-step checkpoints, not a one-step
  trace).
