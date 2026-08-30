# 8-block necessity/sufficiency screen (Stage A, 1 seed): B2, B7, and B4 emerge as distinct candidates

## Background

The phase-paired experiment (`l2_saturation_ft_freeze_phase_paired.md`) closed out continuity as the operative
variable at the 1-32 position scale and found active-position *placement* dominates instead, without identifying
which position(s) matter or why. This experiment splits the `16-271` intervention window into 8 fixed 32-position
blocks and screens each one causally for two distinct roles, kept in separate series because their active-position
budgets differ and are not directly comparable in absolute terms:

- **B1**: `16-47`, **B2**: `48-79`, **B3**: `80-111`, **B4**: `112-143`, **B5**: `144-175`, **B6**: `176-207`,
  **B7**: `208-239`, **B8**: `240-271`.
- **Single-frozen series** (necessity-candidate read): freeze exactly one block, active everywhere else in the
  window (`224` active positions). Uses the existing plain windowed freeze, no new code.
- **Single-active series** (sufficiency-candidate read): freeze the entire `16-271` window, reactivate exactly one
  block (`32` active positions). New `--diagnostic-ft-reactivate-from-position`/`--diagnostic-ft-reactivate-until-
  position` flags (default `0`/`0`, byte-identical to plain windowed freeze when unset; `l2_sample_count` is
  always `>=1` so the default range never matches). 110/110 `sekirei-train` tests pass (2 new: exact single-hole
  reactivation pattern; byte-identical to plain freeze when unset), `fmt`/`clippy` clean.

**Stage A**: 1 seed (`123`, chosen as the most representative/median of `42`/`7`/`123` across final saturated
fraction and `‖h‖` in every prior arm this investigation has measured — `42` trends high, `7` trends low, `123`
sits between them) × 16 arms (8 blocks × 2 series) = 16 runs. Screening pass only, not a confirmed result.

Two onset events kept separate per the standing convention: `T_any` (first live-training-sample `clamped_high`)
and `T_probe_1`/`T_probe_25` (fixed-probe-set first-nonzero / 25%-saturated crossing).

**Empirical validation before trusting any block's read**: chained `‖Δθ_FT‖` between consecutive block-boundary
checkpoints (`16,48,80,...,271`) confirms active movement landed exactly in the intended block for all 16 arms.
Every "should-be-frozen" link shows either exactly `0.0` or a small (`44-65`) 1-position boundary artifact
(checkpoint `posK` reflects state *after* training position `K`, so the link `K1→K2` spans positions `K1+1..K2`,
always including one position from the following block); every "should-be-active" link shows `200-650` — a clean
`4-10x` separation, confirming correct placement in all 16 runs.

## Results

### Necessity-candidate series (single-frozen)

| block | `T_any` | `T_probe_1` | `T_probe_25` | `‖h‖@271` | linear%@271 |
|---|---|---|---|---|---|
| B1 (16-47) | 120 | 128 | 240 | 62.3 | 7.0% |
| **B2 (48-79)** | **161** | **144** | **304** | **54.4** | **31.9%** |
| B3 (80-111) | 140 | 128 | 256 | 63.0 | 9.4% |
| B4 (112-143) | 119 | 128 | 288 | 64.0 | 12.6% |
| B5 (144-175) | 114 | 128 | 144 | 66.4 | 2.2% |
| B6 (176-207) | 114 | 128 | 144 | 66.6 | 2.5% |
| B7 (208-239) | 114 | 128 | 144 | 60.8 | 3.1% |
| B8 (240-271) | 114 | 128 | 144 | 61.2 | 4.5% |

(Control, seed 123, reused from `ft_intermittent_exp/`: `T_any=114`, linear%@271≈3%.)

**B2 stands out**: freezing it alone delays both `T_any` (`114→161`) and `T_probe_25` (`~144→304`), and pushes
linear%@271 an order of magnitude above control (`3%→31.9%`) — the largest disruption of any single block.
B1/B3 show moderate effects; B5-B8 are indistinguishable from control (freezing any of them, alone, changes
nothing measurable) — consistent with saturation onset (`T_any=114`) already occurring before B5 even starts.

**B4 dissociates local firing from propagation**: `T_any` (`119`) is barely different from control, but
`T_probe_25` (`288`) is delayed almost as much as B2's. This is the "`T_any` unchanged, `T_probe_25` selectively
delayed" pattern — a block implicated in probe-set-wide spread without being implicated in first local firing.

### Sufficiency-candidate series (single-active)

| block | `T_any` | `‖h‖@271` | linear%@271 |
|---|---|---|---|
| B1 | 289 | 20.5 | 37.5% |
| B2 | 286 | 28.2 | 40.6% |
| B3 | 288 | 25.3 | 40.6% |
| B4 | 293 | 23.7 | 40.6% |
| B5 (144-175) | 364 | **2.3** | 50.5% |
| B6 | 318 | 12.7 | 42.7% |
| **B7 (208-239)** | 259 | **46.0** | 18.9% |
| B8 (240-271) | 287 | 40.9 | 40.6% |

**B7 stands out**: active alone, it builds `‖h‖@271=46.0` (`~70%` of control's full-window value) — more than any
other single block, including blocks with identical active-position count. B8 is second (`40.9`). **B5 is the
weakest**: active alone, `‖h‖@271=2.3` — essentially no growth, indistinguishable from a fully-frozen baseline.

`T_any` in this series clusters `259-364` for every block, including the weak ones — because position `272`
onward is fully unrestricted for every arm regardless of which block was active, so `T_any` here is dominated by
what happens *after* release, not by the block's own within-window contribution. `‖h‖@271`, measured exactly at
the window boundary before unrestricted training resumes, is the metric that isolates the block's own effect.

### No block is a hotspot on both axes

B2 (necessity-candidate strongest) ranks only 4th of 8 on sufficiency (`28.2`). B7/B8 (sufficiency-candidate
strongest) show zero necessity signal (freezing either, alone, is indistinguishable from control). These are
different blocks carrying different roles, not one block dominating both.

## Terminology, deliberately restricted

Nothing here establishes strict necessity or sufficiency for the underlying saturation mechanism. What Stage A
supports is narrower:

- **B2 is a necessity-*candidate* under control's own trajectory** — freezing it while every other block updates
  normally produces the largest disruption observed. This says nothing about whether B2 would still matter under
  a different trajectory.
- **B7 is a sufficiency-*candidate* under the specific condition that all 7 other blocks are frozen while L2
  keeps updating** — it acts on whatever L2 state has formed by position `208` under that specific (heavily
  frozen) trajectory. This is not evidence that B7's own data content is special in isolation from the state it
  arrives at; block-specific data effects and trajectory-state effects at the time each block executes remain
  entangled.

## Scope

- Necessity and sufficiency series use different active-position budgets (`224` vs. `32`) — their absolute
  metric values are not directly comparable across series; only within-series block-to-block and within-series
  block-to-baseline comparisons are meaningful.
- `T_any` in the single-active series is strongly confounded by the unconditional resumption of FT updates at
  position `272` onward — treated as secondary to `‖h‖@271` for that series, per the above.
- Checkpoint-boundary attribution carries a small (`~1` position, `44-65` in `‖Δθ_FT‖` units) leak into the
  adjacent block's link, disclosed in the validation section, not corrected for (it does not change which block
  is active by more than a factor of `4-10x` margin in any of the 16 arms).
- Data content and trajectory state at the time each block executes are not separated in this design — the
  restricted terminology above exists specifically because of this.
- 1 seed only (`123`) — a screening pass, not a confirmed result. Stage B (3 seeds, `B2`/`B4`/`B7`, plus an
  optional `B5` negative control) is the confirmation step.

## Status

- `--diagnostic-ft-reactivate-from-position`/`--diagnostic-ft-reactivate-until-position`: implemented, tested
  (110/110 `sekirei-train` tests pass, 2 new), `fmt`/`clippy` clean. Committed.
- 16 runs (8 blocks × single-frozen/single-active, seed 123), complete, self-verified, block placement empirically
  validated. Raw data and scripts in scratch (`ft_block_screen_exp/`: `run.sh`, `verify_meta.py`,
  `verify_meta_plain.py`, `satprobe_out/`, `xcross_out/`, `xcross_chain/`).
- Next: Stage B, 3-seed confirmation of B2/B4/B7 (plus optional B5 negative control), before any game-content
  analysis.
