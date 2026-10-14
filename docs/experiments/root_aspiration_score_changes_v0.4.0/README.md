# Initial root aspiration width from completed-score changes

This is an independent, disabled candidate: `T_ASP_VOLATILITY=0` is the
baseline and `1` opts in. It is not combined with the cooperative root split,
helper root LMR, root hints, retirement, NMP prior or any other tuning factor.
NNUE training, weights and labels are unchanged.

One worker keeps the last completed Exact primary root score and an integer
EWMA of its absolute differences within one go:

```
change = abs(score[d] - score[d-1])
ewma = (3 * ewma + change) / 4
initial_delta = clamp(max(ASP_DELTA, 2 * ewma), ASP_DELTA, 4 * ASP_DELTA)
```

The first completed normal score initializes only the previous-score slot.
Mate scores and non-Exact observations invalidate this state. Interrupted
iterations never enter it. Each worker/go owns separate local state. Only
single-PV normal roots are eligible; configured MultiPV and fixed-root APIs
retain the baseline policy. The window centre, all searched depths and the
existing geometric retry widening stay unchanged. The candidate only widens
the initial half-window, and cannot narrow stable positions below ASP_DELTA.

This score-difference rule is an independent hypothesis, not an implementation
of YaneuraOu's statistics. In pinned YaneuraOu source
`c1b80eaa09fe13d5f12b1599d1ae4d53c224de30`, `meanSquaredScore` is a smoothed
signed `value * abs(value)`, not variance about the average. No coefficient in
this candidate was selected from external evaluator output.

## Diagnostics

`AspirationDiagnostics=true` enables a counter-only observer in normal and
Lazy SMP search and prints per-go, per-worker deltas. LazyDiagnostics also
includes these root observations. Normal defaults emit no aspiration records.
The observer has node-counter overhead; it is separate from the timing lane.

Each depth bucket has ten columns: roots, aspirated, adapted, delta_sum,
passes, fail_low, fail_high, retries, retry_nodes, aborted. Bucket 31 includes
31+. delta_sum is only for aspirated roots; divide by aspirated for mean
initial width. A root here means a root-window loop entered after root safety
and terminal checks, not necessarily a completed iterative-deepening step.

Every attempted pass is counted. Aborted passes are not classified as fail
high/low. retries = passes - roots; retry_nodes includes interrupted retry
work and, in the separate cooperative prototype, helper nodes. These nodes
are observed re-search work, not a causal estimate of avoidable waste. The
parser checks depth totals, event consistency and retry nodes <= total nodes.
Snapshots are taken only after participating workers finish. A cooperative
helper has no own root loop and therefore reports a zero root snapshot.

## Validation plan

- Compare the disabled candidate's 1T fixed-depth signatures (bestmove, score,
  PV, nodes, depth) with the previously frozen binary on the full diagnostic
  corpus. Verify observer noninterference separately.
- Exercise actual wider-window activation, bounds and mate reset, MultiPV and
  fixed-root exclusion, go isolation and USI diagnostic enable/disable.
- Measure one factor at 1T and 4T, 300 ms, Hash 64, material-only, no book,
  MoveOverhead 0, fixed quiet/tactical diagnostic positions, per-position ABBA.
- Check all returned PVs. Require a stable depth/work benefit before advancing
  one candidate to an independent material self-play H1 gate. No diagnostic
  score, depth or NPS result by itself establishes playing strength.
