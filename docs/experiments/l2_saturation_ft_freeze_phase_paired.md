# Phase-paired continuity isolation: continuity closed out at the 1-32 position scale, active-position placement dominates

## Background

The intermittent-vs-continuous experiment (`l2_saturation_ft_freeze_continuity.md`) found Intermittent and
Continuous-Late differed in two confounded ways at once — how chunked their active FT updates were, *and* which
specific positions (and therefore which training data) were active vs. frozen. This experiment isolates
continuity from placement directly: for two block sizes, run both possible phase offsets of the same cycle, so
that phase-A's active positions are phase-B's frozen positions and vice versa. If continuity itself governs the
outcome, paired-phase averages should differ by block size. If placement governs the outcome, the phase pairs
themselves should differ sharply within a block size, while paired averages across block sizes stay close.

**Design**: intervention window `16-271` (256 positions, chosen so it divides evenly by both cycle lengths: `256 /
2 = 128` and `256 / 64 = 4`, giving every arm exactly 128 active positions). Two block sizes — fine (`1` active /
`1` frozen) and coarse (`32` active / `32` frozen) — each run with two phase starts:

- **active-first** (today's default): cycle starts active at `from_position`.
- **frozen-first**: cycle starts frozen at `from_position` (new `--diagnostic-ft-frozen-first` flag; for equal
  block lengths this produces the exact complement pattern of active-first — every position active under one is
  frozen under the other).

4 arms (fine active-first / fine frozen-first / coarse active-first / coarse frozen-first) × 3 seeds = 12 runs.
Control reused from the intermittent experiment (byte-identical recipe; `--diagnostic-ft-frozen-first` is a no-op
when no freeze layer is set, proven by unit test). Same dense `--trace-weights --trace-positions` snapshots
(`16-336` in 16-position steps, sparser beyond), `--sample-grad-trace 768`. 108/108 `sekirei-train` tests pass
(2 new: exact-complement pattern for `from=2,until=9,active=2,frozen=2,frozen_first=true`; byte-identical to
active-first default when unset), `fmt`/`clippy` clean.

Per the standing convention, two onset events kept explicitly separate rather than collapsed:

- **`T_any`**: first live-training-sample `clamped_high` (local, first-firing signal).
- **`T_probe_1`/`T_probe_25`**: fixed-261-position-probe-set first-nonzero / 25%-saturated crossing (broad,
  pooled-propagation signal).

## Results

### 1. `T_any`: phase has almost no effect at fine, a huge effect at coarse

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| fine active-first | 161 | 161 | 162 |
| fine frozen-first | 161 | 161 | 161 |
| coarse active-first | 287 | 287 | 287 |
| coarse frozen-first | 161 | 161 | 161 |

Fine's phase gap is `0-1` position, all 3 seeds. Coarse's phase gap is `126` positions (`287` vs. `161`),
identical across all 3 seeds — not sampling noise.

### 2. `‖h‖@271`: same interaction, on a continuous (non-thresholded) measure

| arm | seed 42 | seed 7 | seed 123 | mean |
|---|---|---|---|---|
| fine active-first | 47.84 | 44.53 | 44.49 | 45.6 |
| fine frozen-first | 46.92 | 41.27 | 41.51 | 43.2 |
| coarse active-first | 37.04 | 34.84 | 36.15 | 36.0 |
| coarse frozen-first | 48.46 | 47.36 | 47.59 | 47.8 |

**Paired-average comparison (the primary judgment)**:

| axis | value | gap |
|---|---|---|
| fine phase gap (active-first − frozen-first) | 45.6 vs. 43.2 | `2.4` (`~5%`) |
| coarse phase gap (active-first − frozen-first) | 36.0 vs. 47.8 | `11.8` (`~28%`) |
| block-size gap (fine paired-avg vs. coarse paired-avg) | 44.4 vs. 41.9 | `2.5` (`~6%`) |

Phase within coarse moves `‖h‖` by `~5x` more than block size does once phase is averaged out. **Fine block-length
cycling (1-position alternation) averages away whichever positions matter; coarse block-length cycling (32-position
blocks) does not — an entire block lands on one side of the phase split or the other, and which side it lands on
dominates the outcome.**

### 3. `T_probe_1`/`T_probe_25` kept separate: the firing block and the propagation signal disagree on which arm is "fastest"

| arm | `T_probe_1` | `T_probe_25` | sat%@271 |
|---|---|---|---|
| fine active-first | 271-288 | 288-320 | 0.0-0.4% |
| fine frozen-first | 271-288 | 288-320 | 0.0-0.4% |
| coarse active-first | 288 | 288-304 | 0.0% |
| coarse frozen-first | **144** (all 3 seeds) | 304-320 | 0.4-3.0% |

Coarse frozen-first is the clear outlier on `T_probe_1` (fires far earlier, `144` vs. `271-288` for everyone else)
and on `‖h‖`/`T_any` — but its `T_probe_25` (`304-320`) is **not** the fastest; it lands in the same range as
every other arm. **Firing early (local, `T_any`/`T_probe_1`) and spreading broadly (`T_probe_25`) are not the same
thing** — coarse frozen-first ignites sooner but does not propagate across the probe set faster than the other
arms. This matches the standing observation that live-sample and probe-set-wide onset diverge; here it shows up as
an arm that leads on one axis and is unremarkable on the other, not a single arm dominating both.

### 4. Cumulative `Σ‖Δθ_FT‖`@271 shows block size drives raw movement, not phase — and it's decoupled from `‖h‖`

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| fine active-first | 2115.8 | 2082.3 | 2109.3 |
| fine frozen-first | 2137.1 | 2111.0 | 2137.9 |
| coarse active-first | 1738.3 | 1689.3 | 1723.9 |
| coarse frozen-first | 1745.8 | 1733.8 | 1744.2 |

Phase makes almost no difference here (fine ≈ fine, coarse ≈ coarse) — the opposite pattern from `‖h‖`, where
phase was the dominant axis for coarse. **The clearest single data point against a dose/movement clock in this
whole investigation**: coarse active-first and coarse frozen-first move FT by almost exactly the same total
parameter-space distance (`1738` vs. `1746`, `<1%` apart) yet land at very different `‖h‖` (`36.0` vs. `47.8`,
`~28%` apart). Same amount of movement, very different outcome — it is not how much FT moved, it is when.

### 5. All arms converge to a similar plateau by position 320-512 (deferral, not removal — again)

| position | fine active-first | fine frozen-first | coarse active-first | coarse frozen-first |
|---|---|---|---|---|
| 320 (seed 42/7/123) | 39.8/28.0/33.6 | 40.8/28.7/34.8 | 42.2/26.6/35.7 | 43.7/27.0/35.2 |
| 384 | 40.7/29.2/34.8 | 41.2/29.4/35.3 | 42.3/27.0/36.0 | 46.7/29.3/38.1 |
| 512 | 40.9/29.2/35.1 | 41.2/29.4/35.3 | 42.3/27.0/36.0 | 47.1/29.6/38.1 |

By position `320` the head-start differences visible at `271` have mostly washed out; by `512` all 4 arms sit
within a few points of each other per seed. No arm shows a durably faster *rate* of re-saturation after its
respective release — the differences at `271` are explained by head start, not by an ongoing rate difference.

## Decision-table read

1. **1/1 consistently more stable than 32/32** — not supported. Coarse frozen-first is the single fastest-firing
   arm of all 4 (`T_any=161`, `T_probe_1=144`); chunking updates finely does not confer a general stability
   advantage.
2. **Phase differs sharply, block size differs little** — **supported**, cleanly, on both `T_any` and `‖h‖@271`,
   all 3 seeds.
3. **No difference once phase is paired/averaged** — not supported; the coarse phase gap is real and large.
4. **Coarse specifically relapses faster after release** — not supported as an independent rate effect; see §5.

## Conclusion

Continuity of FT updates is not the operative variable at the block sizes tested here (1 and 32 positions).
Fine-grained alternation (1/1) washes out whichever positions matter, leaving almost no phase effect; coarse
alternation (32/32) does not, and phase alone moves `T_any` by 126 positions and `‖h‖@271` by ~28% — far more than
switching block size while pairing out phase (~6%). Active-position placement, not update continuity, is the
dominant axis at this scale.

**This does not yet identify *why* placement matters.** Active/frozen placement changes not only which raw
training data was seen, but also the FT/L2 state entering every subsequent block — an arm that was active in an
early block enters the next block from a different weight trajectory than an arm that was frozen there. Data
content and trajectory state are still entangled; this experiment closes the continuity question but does not
by itself attribute the effect to specific game content. That is deferred to a causal block-localization pass
(next experiment) before any game-content analysis is undertaken.

## Scope

3 seeds, 4 arms, same dense-snapshot schedule as the intermittent experiment. `Σ‖Δθ_FT‖` remains a chained-sparse-
snapshot lower-bound approximation (§4). `‖w‖`/`cos(h,w)` were not separately re-tabulated here (already
established as comparatively stable/uninformative relative to `‖h‖` in the prior two experiments); not re-checked
for regressions in this specific run.

## Status

- `--diagnostic-ft-frozen-first`: implemented, tested (108/108 `sekirei-train` tests pass, 2 new), `fmt`/`clippy`
  clean. Committed.
- 12 runs (fine/coarse × active-first/frozen-first × 3 seeds), complete, self-verified; Control reused from
  `ft_intermittent_exp/`. Raw data and scripts in scratch (`ft_phase_paired_exp/`: `run.sh`, `verify_meta.py`,
  `satprobe_out/`, `xcross_out/`, `xcross_chain/`).
- Next: causal 8-block (`16-47` ... `240-271`) necessity/sufficiency screen (single-active / single-frozen series)
  to localize which specific 32-position block(s) drive the effect, before any game-content analysis.
