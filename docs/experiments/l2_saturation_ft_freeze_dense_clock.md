# Dense-snapshot clock re-measurement: sample-count and update-dose clocks ruled out by elimination; the physical-state hypothesis survives, but not by direct confirmation

## Background

The localization experiment (`l2_saturation_ft_freeze_localization.md`) found Early (`16-64` frozen), Late
(`64-128` frozen), and Full (`16-128` frozen) all relapse into saturation within an identical `127-191`
positions-after-release bracket — but that bracket came from linear interpolation across the `256→320` checkpoint
gap, which spans nearly the *entire* `0%→45%` saturation ramp with no snapshot in between. An `~18%` cross-arm
spread measured inside that gap isn't trustworthy precision. This experiment re-runs the same 4 arms (Control,
Early, Late, Full) with dense `224/240/256/272/288/304/320/336` snapshots (16-position steps) specifically to
close that gap, and keeps three onset definitions separate rather than collapsing them into one, per the user's
explicit framework:

- **`T_any`**: the first live-training position (from `--sample-grad-trace`'s per-position `l2_gate`) where *any*
  neuron reads `clamped_high` on the specific board actually being trained on.
- **`T_probe_1`**: the first fixed-261-position-probe-set checkpoint with nonzero pooled saturated fraction.
- **`T_probe_25`**: the (now densely-interpolated) point where the fixed probe set's pooled saturated fraction
  crosses 25%.

**Design**: 12 runs (Control/Early/Early/Late/Full × 3 seeds — 4 arms), same recipe as the localization experiment,
`--trace-weights --trace-positions 16,32,48,64,80,96,112,128,160,192,224,240,256,272,288,304,320,336,384,512`. Self-
verified metadata against requested config for all 12 runs before use. New `l2_threshold_crossing_probe.rs`
computes, per checkpoint pair, `‖h_release‖`/`‖h_target‖`/`‖Δh‖` (release-anchored), per-neuron `‖w‖`, `cos(h,w)`,
actual `z` (pre-activation), and `dot_no_bias = z - bias` (`= ‖h‖·‖w‖·cos(h,w)`), plus a scalar raw FT
parameter-space distance `‖Δθ_FT‖` between the two checkpoints. **Verified before trusting**: cross-checked against
the already-verified `l2_alignment_formation_probe.rs` on an identical checkpoint pair — `norm_h_release`,
`cos_target`, and `z_target` matched to the reported precision exactly (`14.09232`/`0.7835886`/`48.763477` in
both tools). Cumulative `Σ‖Δθ_FT‖` is approximated by chaining `‖Δθ_FT‖` across consecutive *sparse* snapshots
(release onward) — a lower-bound approximation of true path length (undercounts any back-and-forth movement
between snapshots), disclosed rather than corrected for.

## Results

### 1. Three onset events, kept separate — they behave differently, not interchangeably

| arm | release | `T_any` (+after release) | `T_probe_1` (+after release, sat%) | `T_probe_25` (+after release, interpolated) |
|---|---|---|---|---|
| Early (16-64) | 65 | 161 (+97, all 3 seeds identical) | 160 (+96 to +96, sat% 0.4-1.7%) | +218 to +237 |
| Late (64-128) | 129 | 287 (+159, all 3 seeds identical) | 256-272 (+128 to +144, sat% 0.3-1.8%) | +155 to +182 |
| Full (16-128) | 129 | 292-303 (+164 to +175) | 288-304 (+160 to +176, sat% 0.2-21.8%) | +159 to +188 |

`T_any` and `T_probe_1` track each other closely within each arm (both are "first local saturation" signals, one
on a live single sample, one on the fixed probe set) but diverge substantially *between* arms in release-relative
terms — no clean single-clock story fits all three numbers for either. `T_probe_25` shows Early consistently
taking longer, release-relative, to reach broad (25% of probe set) saturation than Late or Full — a genuine,
3/3-seed-consistent pattern, not resolved further here. **Full's `T_probe_1` is poorly resolved**: its pooled
saturated fraction jumps from `0%` directly to `16.6%`/`21.8%` in a single `16`-position step (seed 7/123) — the
true first-crossing point is somewhere inside that gap, not exactly at the reported checkpoint; noted, not chased.

### 2. Sample-count and update-dose clocks ruled out by elimination, paired by seed at `T_probe_25`

| seed | axis | Early | Late | Full | cross-arm spread |
|---|---|---|---|---|---|
| 42 | positions-after-release | 218.1 | 154.6 | 173.1 | 34.9% |
| 7 | positions-after-release | 236.6 | 181.9 | 188.1 | 27.1% |
| 123 | positions-after-release | 223.6 | 159.2 | 179.7 | 34.4% |
| 42 | cumulative `Σ‖Δθ_FT‖` | 2506 | 1930 | 2070 | ~25% |
| 7 | cumulative `Σ‖Δθ_FT‖` | 2735 | 2179 | 2236 | ~20% |
| 123 | cumulative `Σ‖Δθ_FT‖` | 2592 | 1976 | 2132 | ~24% |

Both axes show substantial, consistently-signed (Early always highest) cross-arm spread at a matched onset event,
in all 3 seeds — **neither is a clean shared clock**. This directly rules out both the "elapsed sample count since
release" hypothesis and the "cumulative FT update magnitude" (active-update-dose) hypothesis as the thing that
determines when `T_probe_25` occurs.

### 3. The product test: not the confirmation it first looked like — flagged before writing it as one

The natural next check — does `‖h‖·‖w‖·cos(h,w)` (the actual pre-activation-minus-bias, the literal quantity
being compared against the `127` threshold) converge across arms at `T_probe_25` — initially looked like a clean
win: `2.5-5.0%` cross-arm spread, paired by seed, tight in all 3 seeds. **This was caught before writing it up as
"threshold-crossing confirmed."** The problem: `T_probe_25` is *itself defined* by the pooled `z` (which this
product reconstructs, `dot_no_bias = z - bias`) crossing the saturation threshold. Measuring "does `z-bias` land
near a fixed value at the moment `z` is defined to have crossed a fixed value" is close to definitional — a tight
product spread at a `z`-defined event is close to guaranteed regardless of which clock actually governs *when*
that event happens, and is not independent evidence for the physical-state hypothesis. **The physical-state
reading survives by elimination (§2), not because the product number itself confirms it.**

**What is genuine, non-circular content in the product measurement**: it's computed on *control's own*
saturating-neuron set specifically, and lands in a similar range (`~127-163`, seed-dependent, but consistent
within a seed across all 3 arms) regardless of which window was frozen or when release happened. That means the
*same neurons* cross, at a *similar pre-activation distribution shape*, under Early, Late, and Full alike — a
co-adaptation-target invariance: perturbing *when* FT resumes moving doesn't change *which* neurons end up
saturated or their approximate crossing level, only *when* they cross. That's a real finding, reported as that,
not as a magnitude-based confirmation of a specific clock.

## Decision-table read, corrected

The user's pre-registered framework treated "product converges tightly" as strong, standalone support for
physical threshold-crossing. Given the circularity above, that specific branch is downgraded: **product
convergence is not being used as evidence here.** The actual support for the physical-state reading is
elimination-based: sample-count and cumulative-update-dose clocks both show real (`~20-35%`, non-circular)
cross-arm variance and are ruled out; nothing else was tested that survives as a competing clock. This is weaker
than the user's originally-registered "strongly support" bar for a converging product, and is stated as such.

**This revives the intermittent-freeze follow-up, which the user's own plan treated as conditional on the product
test being decisive.** Since it wasn't (circularity, not a clean win), the intermittent-freeze arm (`16` positions
FT-active / `16` positions frozen, repeated, vs. a continuous-update arm matched on total active-FT-position
count) remains a genuinely non-circular test — it doesn't rely on any quantity defined in terms of the saturation
threshold itself, and would directly distinguish "total active FT movement, however chunked, governs onset" from
"continuity/momentum matters, chunked movement is less effective than the same total continuous movement."

## Scope

3 seeds, 4 arms, dense snapshots restricted to `224-336`; earlier and later checkpoints remain at the original
sparse spacing. `Σ‖Δθ_FT‖` is a chained-sparse-snapshot approximation of true path length, not measured at every
training position. `T_any`/`T_probe_1` are reported as observed, not fit into a single leading-term narrative —
the data doesn't cleanly support one.

## Status

- `l2_threshold_crossing_probe.rs`: implemented, verified (exact match against `l2_alignment_formation_probe.rs`
  on an identical checkpoint pair), `fmt`/`clippy` clean. **Uncommitted.**
- 12 runs (Control/Early/Late/Full × 3 seeds), dense `--trace-weights` snapshots, self-verified metadata,
  complete. Raw data and scripts in scratch (`ft_dense_clock_exp/`: `run.sh`, `verify_meta.py`,
  `analyze_dense_clock.py`, `satprobe_out/`, `xcross_out/`, `xcross_chain/`).
- Intermittent-freeze arm: not yet run, now recommended rather than optional, per the corrected product-test
  reading above.
