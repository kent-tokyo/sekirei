# Intermittent vs. continuous FT freeze: no evidence for a continuity effect, but the arms are also not continuity-only tests

## Background

The dense-clock experiment (`l2_saturation_ft_freeze_dense_clock.md`) eliminated the sample-count and
cumulative-update-dose clocks by elimination, and flagged that its product-convergence test was near-circular
(measured at a `z`-defined event). That left the intermittent-freeze arm as the first genuinely non-circular test
of whether *continuity* of FT updates — not just their total count — matters for how quickly L2 saturates.

**Design**: intervention window `position 16-271` (256 positions), all 3 experimental arms get exactly 128
FT-active positions inside that window:

- **Intermittent**: `16` active / `16` frozen, repeating (`--diagnostic-ft-active-block 16
  --diagnostic-ft-frozen-block 16`).
- **Continuous-Early**: `16-143` active, `144-271` frozen.
- **Continuous-Late**: `16-143` frozen, `144-271` active.
- **Control**: full run, no freeze.

3 seeds (42/7/123) × 4 arms = 12 runs, dense `--trace-weights --trace-positions` snapshots from `16` through `768`
(16-position steps through `336`, sparser beyond), `--sample-grad-trace 768`. New trainer support:
`--diagnostic-ft-active-block`/`--diagnostic-ft-frozen-block` (both default `0`, byte-identical to the plain
single-window freeze when unset; only takes effect when `--diagnostic-freeze-layer ft`). Self-verified metadata
(including the new block-length fields) against requested config for all 12 runs before use. 106/106
`sekirei-train` tests pass, `fmt`/`clippy` clean.

Onset kept as three separate events throughout, per the standing convention (`T_any`, `T_probe_1`, `T_probe_25` —
see `l2_saturation_ft_freeze_dense_clock.md` for definitions).

## Results

### 1. Continuous-Early is a control check, not an informative continuity arm

Continuous-Early is frozen only for `144-271` — i.e., it follows the exact same FT trajectory as Control through
position `143`. Since Control's own `T_any` (`109-114`, all seeds) already falls before `143`, Continuous-Early
has *already started saturating before its freeze window begins*. Its results confirm the freeze mechanism is
inert until engaged (matches Control almost exactly on every metric below) but it does not test continuity or
placement — it is a sanity/positive-control arm.

| arm | `T_any` | `T_probe_1` | `T_probe_25` | `‖h‖@271` | linear neurons@271 (/32) |
|---|---|---|---|---|---|
| Control | 109-114 | 112-128 | 144-288 | ~64-66 | ~1 |
| Continuous-Early | 109-114 | 112-128 | 144-304 | ~61-62 (~95% of Control) | ~1-2 |

### 2. The informative comparison — Intermittent vs. Continuous-Late — shows no continuity advantage

| arm | `T_any` | `T_probe_1` | `T_probe_25` | `‖h‖@271` | linear neurons@271 (/32) | cumulative `Σ‖Δθ_FT‖`@271 |
|---|---|---|---|---|---|---|
| Intermittent | 288-291 | 288 (all seeds) | 320-336 | ~27-28 (~42% of Control) | ~9-14 | ~1868-1916 |
| Continuous-Late | 317-320 | 304-320 | 320-336 | ~16-17 (~26% of Control) | ~12-18 | ~1368-1428 |

If continuity (short-term FT-L2 feedback, uninterrupted gradient sequences, or optimizer-state momentum) were the
operative variable, the fully-consecutive Continuous-Late arm — 128 uninterrupted active positions ending exactly
at the measurement point — should saturate faster or reach a higher `‖h‖` than the same 128 positions chopped into
16-position chunks. It does not: Continuous-Late's `T_any` is later and its `‖h‖@271` is lower than Intermittent's,
the opposite of what a continuity-matters hypothesis predicts. Intermittent also moved its FT parameters
*further* in aggregate (`Σ‖Δθ_FT‖ ≈ 1900` vs. `≈ 1400`) while ending up with lower saturation than
Continuous-Early — more cumulative movement did not translate to more effective norm growth, reinforcing that raw
update magnitude ("dose") is not the governing clock either, consistent with the dense-clock experiment's earlier
elimination of that hypothesis.

**This does not establish "continuity is irrelevant."** Intermittent and Continuous-Late differ in two ways at
once — how chunked their active updates are, *and* which specific positions (and therefore which training data)
were active vs. frozen. The two are confounded in this design; only a phase-paired complement (next experiment)
can separate them.

### 3. Early-active fraction correlates with `‖h‖@271` far more cleanly than any continuity measure

| arm | % of active budget in position 16-143 | `‖h‖@271` (% of Control) |
|---|---|---|
| Continuous-Early | 100% | ~95% |
| Intermittent | ~50% | ~42% |
| Continuous-Late | 0% | ~26% |

Roughly monotonic across all 3 arms and all 3 seeds. Near-threshold neurons in Intermittent already show
alignment comparable to Control's saturated set (`cos(h,w) ~0.87-0.89` at `z~105-108`, seed 42) despite `z` still
being below the `127` ceiling — consistent with the windowed-freeze finding that alignment forms largely
independent of freezing, and that `‖h‖` growth (not alignment) is the gating factor. Continuous-Late's near-
threshold neurons lag on both axes (`cos~0.76-0.82` at `z~70`).

This is a real, non-circular finding, but it is a statement about **where in the window the active budget falls**,
not a clean causal statement about *why* — position and training-data content are confounded (per the standing
scope note: game content has not yet been examined).

### 4. All arms relapse; deferral, not removal

Every suppressed arm (Intermittent, Continuous-Late) eventually reaches comparable saturated fractions to Control
within roughly the same number of positions after exhausting its active-FT budget (`T_probe_1` at +17 to +49
positions past `271`) — the same "deferral, not removal" pattern seen in every previous freeze experiment this
investigation has run. No arm shows permanent suppression.

### 5. Unexplained secondary observation: transient decline at position 640-768

Control and Continuous-Early are perfectly flat (saturated fraction unchanged to the decimal place) from position
`384` through `768` in every seed — neurons that saturate early stay saturated. Intermittent and Continuous-Late,
by contrast, both show a temporary *decline* in saturated fraction somewhere in the `416-768` range after their
initial relapse peak (e.g. Intermittent seed 42: `40.6%@384 → 27.8%@448 → 38.9%@512 → 23.9%@768`, non-monotonic
throughout) before partially recovering. The cleanest available reading is that recently-saturated neurons can
still recede while long-saturated ones are locked in — not a property of any specific absolute position range,
since Control/Continuous-Early pass through the same `416-768` positions without any decline. This is flagged as
an open observation, not investigated further here.

## Decision-table read

Mapped against the pre-registered 5-branch framework:

1. **Intermittent clearly more stable than both continuous arms** — not supported. Continuous-Late is *more*
   suppressed than Intermittent on both `‖h‖` and `T_any`.
2. **Intermittent between Early and Late** — supported on `‖h‖`/`T_any` (continuous, graded measures); not cleanly
   supported on `T_probe_1`/`T_probe_25` (pooled probe-set measures), where Intermittent and Continuous-Late are
   close together. Resolution-dependent, consistent with the earlier finding that live-sample and probe-set-wide
   onset are not interchangeable.
3. **All 3 arms converge to a similar state at 271** — not supported; Continuous-Early tracks Control closely,
   Intermittent and Continuous-Late are both clearly behind it.
4. **Intermittent stays stable long after release** — not supported; Intermittent's `T_probe_1` (288) precedes
   Continuous-Late's (304-320).
5. **Relapses on the same timescale as before (deferral, not removal)** — supported; see §4.

## Conclusion

Intermittent FT updates showed no evidence of specially suppressing saturation compared to continuous updates.
The observed differences tracked how much of the intervention window's first half (`16-143`) carried active FT
updates far more cleanly than any continuity measure. However, update placement is confounded with training-data
content, so continuity's effect in isolation remains undetermined by this experiment. Continuous-Early functions
as a control check (its freeze window opens after Control's own saturation has already begun) rather than an
informative continuity arm — the real comparison is Intermittent vs. Continuous-Late, and it points away from
continuity as the operative variable without fully ruling it out.

## Scope

3 seeds, 4 arms, dense snapshots `16-336` in 16-position steps, sparser beyond. `Σ‖Δθ_FT‖` is a chained-sparse-
snapshot approximation (lower bound, undercounts back-and-forth movement) — the ~35% gap between Intermittent and
the continuous arms has not been verified against a denser chain and could partly be a measurement artifact of
snapshot spacing relative to the 16-position cycle length. Game content is not examined here (per standing scope).

## Status

- `--diagnostic-ft-active-block`/`--diagnostic-ft-frozen-block`: implemented, tested (106/106 `sekirei-train`
  tests pass), `fmt`/`clippy` clean. Committed.
- 12 runs (Control/Intermittent/Continuous-Early/Continuous-Late × 3 seeds), complete, self-verified. Raw data and
  scripts in scratch (`ft_intermittent_exp/`: `run.sh`, `verify_meta.py`, `satprobe_out/`, `xcross_out/`,
  `xcross_chain/`).
- Next: phase-paired continuity isolation (1/1 vs. 32/32 active/frozen block sizes, each run active-first and
  frozen-first, to cancel the position/data-content confound noted in §2-3) — not yet run.
