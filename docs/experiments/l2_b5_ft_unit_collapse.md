# B5 is not an anti-growth block — it is a radial FT-unit collapse

## Background

The block-screening experiments (`l2_saturation_ft_freeze_block_screen_stage_b.md`) found B5 (`144-175`) active
alone produces `‖h‖@271 ≈ 2.2-2.3`, *below* the `all-frozen` floor (`~14`, itself just `‖h‖` at position `16`,
the pre-window release point — all-frozen genuinely moves nothing). This experiment establishes the geometry of
that shrinkage directly, before treating B5 as a usable "anti-growth" mechanism.

**Design**: fine-grained snapshots (`136,143,144,148,152,160,168,175,176,192,208`, merged into the standard dense
grid) for the existing `single_active_B5` arm (B5 active, everything else in `16-271` frozen), 3 seeds. New
probe fields on `l2_alignment_formation_probe.rs` (already had raw `h_old`/`h_new`/`Δh` internally, extended
rather than rebuilt): `cos_h_old_delta_h` (radial direction cosine), `radial_projection` (`Δh · normalize(h_old)`,
signed length along `h_old`'s own direction), `orthogonal_component` (`‖Δh − radial_projection·normalize(h_old)‖`,
the rotation/direction-change part), `dead_units_old`/`dead_units_new` (count of the `2·L1=512` FT-output units
sitting at exactly `0.0`, i.e. clamped at the floor). All pooled over the full 261-position probe set, not a
single board.

**Determinism check before trusting the fine snapshots**: the fine re-run's coarse checkpoints
(`144/160/176/192/208`) are byte-identical (`cmp -s`) to the original Stage A/B `single_active_B5` checkpoints, in
all 3 seeds — confirms the fine re-run reproduces the same trajectory, just at higher time resolution.

## Results

### The B5-block movement (position 143→176) is radial, not rotational, in every seed

| seed | `cos(h_before, Δh)` | radial | orthogonal | dead units @143 | dead units @176 |
|---|---|---|---|---|---|
| 42 | **-0.940** | -9.10 | 2.44 | 6.81 | **217.9** |
| 7 | **-0.941** | -9.09 | 2.47 | 6.13 | **247.3** |
| 123 | **-0.936** | -8.88 | 2.53 | 6.30 | **236.7** |

`cos(h_before, Δh) ≈ -0.94` in all 3 seeds — the update points almost exactly opposite `h_before`'s own direction.
Radial:orthogonal magnitude ratio ≈ `3.6:1` — over `90%` of the movement shrinks `‖h‖` directly; rotation is a
minor secondary component.

### Dead FT units jump from ~1% to ~43-48% of all 512 units

Out of `2·L1 = 512` total FT-output units (both perspectives), the count sitting at exactly `0.0` (clamped at the
floor) goes from `6-7` (`1.2-1.4%`) at position `143` to `218-247` (`42.6-48.3%`) at position `176`, in every seed.
This is not a smooth norm reduction — it is a large fraction of the FT representation being driven into the dead
zone.

### Negative control: all-frozen shows exactly zero change over the identical window

| seed | dead units @143 (frozen) | dead units @176 (frozen) | `‖Δh‖` |
|---|---|---|---|
| 42 | 6.81 | 6.81 | 0.0000 |
| 7 | 6.13 | 6.13 | 0.0000 |
| 123 | 6.30 | 6.30 | 0.0000 |

Exact match to B5-active's own `dead units @143` values (both arms are identical up to position `143` by
construction) and exactly zero movement when frozen, all 3 seeds — confirms the collapse above is specific to
B5's active block, not an artifact of the probe tool or of that absolute position range.

## Conclusion

**B5 is not a mild anti-growth block. It drives a radial collapse of the FT representation, clamping roughly
40-48% of all FT output units to zero within its own 32-position window.** This is the same *kind* of failure
mode (unit collapse toward a ClippedReLU floor) as the L2 saturation collapse this entire investigation exists to
study, but on the *opposite* wall (dead, not saturated) and in a different layer (FT, not L2). Whether this is a
useful regularization effect or a pathological collapse specific to this data/trajectory is not yet determined —
explicitly not classified as "anti-saturation curriculum material" until the driving mechanism is understood.

## Scope

Radial/orthogonal decomposition and dead-unit counts are pooled over the fixed 261-position probe set, not
computed on live training data. `L1=256` (so `2·L1=512` total FT-output units). Why the movement is so strongly
radial (rather than, say, a targeted rotation away from specific saturating neurons' L2 directions) is not
investigated further here — the next step is CP/WDL causal decomposition, not a deeper geometric analysis.

## Status

- `l2_alignment_formation_probe.rs`: extended with `cos_h_old_delta_h`/`radial_projection`/
  `orthogonal_component`/`dead_units_old`/`dead_units_new`, `fmt`/`clippy` clean. Committed.
- Fine-grained snapshots for `single_active_B5`, 3 seeds, complete, self-verified, determinism-checked against the
  original coarse run. Raw data and scripts in scratch (`ft_b5_anti_growth_exp/`: `run.sh`, `verify_meta.py`,
  `align_out/`).
- Follow-up chain: CP/WDL causal decomposition via counterfactual replay (`l2_b5_cp_wdl_component_replay.md`) found
  CP pushes FT backward and WDL reverses sign; a subsequent one-step shadow trace (`l2_b5_shadow_trace.md`) found
  the deep Blended collapse is cross-step accumulation, not within-step interaction — B5 is closed out as a
  documented pathology, not pursued as usable curriculum material.
