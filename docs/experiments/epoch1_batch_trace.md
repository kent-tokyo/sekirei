# Epoch-1 batch-level trace: data order, not initialization, is the dominant driver

## Background

Three closed investigations (gradient clipping, LR warmup, L2 bias init — `docs/experiments/`) each ruled out one
explanation for the epoch-1 L2 dead-neuron collapse without fixing it, all using only epoch-boundary snapshots.
`--trace-positions` (this session) adds per-position instrumentation: L2/FT's joint per-neuron state (pre-activation
split into weighted-input vs. bias components, dead/saturation frequency, gradient mean/RMS/sign-consistency,
applied-update norm) at requested position-counts within an epoch. This experiment uses it for the two-stage
isolation the L2-bias-init writeup's "no bucket fits" conclusion called for next: is the collapse driven by
initialization, or by which specific positions land early in the training order?

**Fixed across every run**: `data/gateA_csa_subset`, `--split-seed 42`, `--wdl-lambda 0.7 --label-depth 4
--min-rate 1500 --lr-schedule cosine --min-lr 0.00001 --warmup-epochs 1 --epochs 1 --l2-bias-init 0.5` (default),
shared teacher-search cache, `--trace-positions 0,1,2,4,8,16,32,64,128,256`.

**Stage 1**: `--init-seed` ∈ {42, 7, 123}, no `--shuffle-seed` (original file order — matching every prior
experiment in this investigation, for continuity).

**Stage 2**: `--init-seed 7` fixed (the most reliably-collapsing seed across every prior experiment),
`--shuffle-seed` ∈ {101, 102, 103} — a new flag added alongside the trace tool (`trainer::shuffled_order`, a seeded
Fisher-Yates over `0..n`, reshuffled fresh each epoch from `seed ^ epoch`). Off by default (`None` = original file
order, byte-identical to every run before this flag existed).

## Stage 1: initialization varies, order fixed

| seed | epoch-end `l2_dead` | trace: full-dead at pos 1 → pos 256 |
|---|---|---|
| 42 | 2 | 9 → 9 → 9 → 9 → 9 → 9 → 9 → 9 → **9** (never recovers within the trace window) |
| 7 | 5 | 7 → 7 → 7 → 7 → 7 → 6 → 6 → 5 → **5** |
| 123 | 3 | 8 → 6 → 6 → 4 → 3 → 3 → 3 → 3 → **3** |

Epoch-end counts match every prior experiment in this investigation exactly (reproducibility confirmed). Different
seeds recover by different amounts, at different rates, within the same unshuffled order — some init-dependence is
real, but the recovery is always partial and gradual; none reaches zero.

## Stage 2: initialization fixed, order varies

| shuffle seed | epoch-end `l2_dead` | trace: full-dead at pos 1 → pos 256 | `l2_input_norm` at pos 256 |
|---|---|---|---|
| 101 | 5 | 7 → 7 → 6 → 6 → 6 → 5 → 5 → 5 → **5** | 36.11 (runaway, matches Stage 1's pattern) |
| 102 | **0** | 7 → 7 → 6 → 6 → 6 → 6 → 4 → 1 → **0** | 12.17 (stayed tame) |
| 103 | **0** | 7 → 7 → 7 → 7 → 7 → 6 → 6 → 2 → **0** | 17.30 (stayed tame) |

**The headline result**: two of three shuffle seeds reach *zero* dead neurons by position 256 — an outcome no
init-seed variation in Stage 1 reached at all, with the identical initialization and identical data, differing
only in the order positions were visited. Shuffle 101 is the exception: its final dead set (`{3, 10, 12, 17, 31}`)
overlaps 4-of-5 with Stage 1 seed 7's own stuck set (`{3, 10, 17, 27, 31}`) and shows the same saturation-runaway
pattern (18 neurons past 30% saturation frequency vs. 0 for shuffle 102/103) — **not every reshuffle rescues the
collapse, only specific orderings do**. Read as: order matters more than init does, but it's which *specific*
early positions a given order happens to front-load, not shuffling in general.

## Per-neuron mechanism: the stuck neurons never receive a single nonzero gradient

Comparing Stage 1 seed 7 (stuck at 5 dead) against Stage 2 shuffle 102 (same init, recovers to 0) at the level of
the 5 neurons stuck under the unshuffled order (`3, 10, 17, 27, 31`):

**Under the unshuffled order, all 5 have `gradient_rms == 0.0` at every single trace point through position 256** —
their `bias` field reads exactly `0.5000` (the untouched init value) at every snapshot. Not "small gradient,"
*zero* gradient, at every position sampled. This is `d_l2_acc[o] == 0` on every one of the first 256 positions —
ClippedReLU's zero-gradient dead zone, exactly as the mechanism has been described throughout this investigation,
now directly observed rather than inferred from before/after endpoints.

**Under shuffle 102, the same 5 neurons "wake" at different, identifiable positions**:

| neuron | last frozen (`bias==0.5000` exactly) | first nonzero gradient observed by |
|---|---|---|
| 3 | position 64 | position 128 (`bias` jumps to 0.5603, `dead_frequency` drops 1.0→0.555) |
| 10 | position 128 | position 256 (`bias` 0.4884, `dead_frequency` 1.0→0.965, barely started) |
| 17 | position 64 | position 128 (`bias` 0.5079, `dead_frequency` 1.0→0.914) |
| 27 | position 64 | position 128 (`bias` 0.5563, `dead_frequency` 1.0→0.523) |
| 31 | position 32 | position 64 (`bias` 0.4985, but `dead_frequency` stays ~0.98 through 256 — see caveat below) |

Every wake-up happens between positions 32 and 256 — inside the traced window, at different times per neuron. Since
the identical neurons under the identical initialization never wake at all across the same window when the data
order is unchanged, **the wake-up event is a property of which specific position lands early in the shuffled
sequence, not of the neuron's initialization**. Some specific input pattern pushes that neuron's weighted-input
term across the zero boundary; whether that pattern appears early enough in this epoch to matter is exactly what
shuffling changes.

**Caveat, not smoothed over**: neuron 31's `gradient_sign_consistency` locks to exactly `1.000` from position 64
onward (every nonzero gradient it receives pushes the same direction) — but its `dead_frequency` barely moves
(0.984 → 0.969 → 0.984, effectively flat) and its `bias` actually drifts *down* slightly (0.4985 → 0.4849). Sign
consistency alone doesn't guarantee recovery; a consistent push in the wrong direction (further from, not toward,
the active zone) doesn't help. Not resolved here — noted for whoever picks up gradient-direction analysis next.

## Applying this to the pre-registered mechanism sketch

The mechanism proposed alongside this experiment's design matches what's observed:

- **Fully dead**: pre-activation pinned at/below the ClippedReLU floor → gradient exactly 0 → neither bias nor
  weight can update → recovery is impossible unless the input distribution happens to cross the boundary on its
  own — confirmed directly (`gradient_rms == 0.0`, `bias` frozen to the exact init value, for every stuck neuron
  at every traced position).
- **Near-dead / recovering**: occasional activation → occasional gradient, which for most (not all) recovering
  neurons pushes consistently toward the active zone — confirmed for 4 of 5 (neuron 31 is the counterexample: rare
  but consistent gradient that doesn't reduce dead frequency).
- **Order-dependence**: which positions are "occasional" enough to cross the boundary, and how early they appear,
  is the shuffle seed's entire effect — confirmed by the same neuron IDs staying frozen under one order and waking
  at specific, different positions under another.

## Status

- `--shuffle-seed` stays in the codebase as an optional flag, default unset (original file order, byte-identical
  to every run before it existed). New unit tests confirm `shuffled_order` is a true permutation, deterministic
  per seed, and differs across seeds.
- **Data order is now the stronger candidate mechanism**, ahead of initialization — Stage 1's three init seeds
  never reached zero dead neurons within the trace window; two of Stage 2's three shuffle seeds did, with the
  identical initialization.
- **Not a promotion decision** — `--shuffle-seed` isn't a training-recipe lever (there's no principled "good"
  shuffle seed to standardize on; the finding is about *why* the collapse happens, not a fix for it). No bucket
  framework applies here for the same reason it didn't fully apply to the L2-bias-init result: this is a diagnostic
  finding, not a candidate default.
- **Next, per the pre-registered priority order**: CP/WDL gradient decomposition (running the backward pass a
  second time with each teacher signal alone, already scoped as cheap in the original explore pass) — now better
  motivated, since the neuron-selection question ("which neurons to look at") is resolved by this experiment: the
  specific-position-dependent stuck/wake pattern is exactly what a directional (not just magnitude) gradient
  decomposition should be pointed at next.
- Full per-epoch, per-position trace data: `trace_multiseed/` in scratch (6 runs: 3 Stage 1, 3 Stage 2).
