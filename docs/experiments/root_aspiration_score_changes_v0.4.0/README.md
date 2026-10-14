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

## Initial validation

Source `d15b8f7`, release/tune binary SHA-256
`7383f9375fc5b7b7be15050fadd603aa554d318ca78c89c62831a67ea5ec2a70`.
The disabled candidate matched the previous frozen executable on all 20
one-thread depth-8 ABBA identity blocks: bestmove, score, PV, nodes and depth.
Rust regression: 456 passed, 7 ignored; the isolated adaptive-policy contract
also passed. Python tooling: 72 passed. Full-workspace tune Clippy and WASM
check passed. Full coverage was not remeasured.

The four-position, depth-8 counter diagnostic completed 32 searches across
1T and 4T. In 1T, six of 56 aspirated roots used a wider initial window;
retries fell from 18 to 12 and retry nodes from 28,350 to 11,772. In 4T,
20 of 200 aspirated roots widened; retries were 85/52 and retry nodes
64,050/31,582 for baseline/candidate. These are diagnostic work observations;
4T root counts and aborts differ, and these totals are not a strength result.
Policy-changing fixed-depth diagnostics explicitly observe identity without
requiring identical trees. The separate disabled-identity gate stays strict.

The initial diagnostic attempt exposed an existing checker error: 1T has no
helper-retirement channel. The checker now accepts its absence only in 1T and
still requires all NMP/aspiration worker records. A dedicated regression test
covers this. That incomplete attempt was not pooled with the successful run.
The successful counter run predates the manifest's measurement-source hashes
and identity_policy field. Its CLI used --policy-diagnostic and the summary
records identity as observed, not required. Subsequent timing manifests include
those provenance fields. The native executable hash is identical throughout.

## Completed 300 ms timing screen

Each qualified lane used 20 fixed diagnostic positions (10 quiet, 10 tactical),
two repeats, 40 per-position ABBA blocks and 160 searches. The same frozen tune
binary was used on both sides; only T_ASP_VOLATILITY=0/1 differed. Material
evaluation, ordinary Lazy SMP (LazyFlags=15), Hash 64, MoveOverhead 0 and
SpecTopN 0 were fixed. Optional node counters were disabled.

| Threads | Median paired depth delta | A-first / B-first delta | Median paired wall-NPS change | Worker 0 depth delta |
| --- | ---: | ---: | ---: | ---: |
| 1 | +1 ply | +1 / +1 | -0.54% | +1 ply |
| 4 | +0.5 ply | +1 / 0 | -1.42% | 0 ply |

These are medians of per-block B-minus-A mean depths, not differences of raw
side medians. Quiet/tactical subgroup medians were 0/+1 ply in 1T and
0/+0.5 ply in 4T. The positive signal is concentrated in tactical positions.
The 4T order directions disagree, so a stable multiworker benefit has not been
established. NPS decreased while completed depth increased; no Elo follows
from either measurement. The configured search budget was 300 ms; observed
wall time includes protocol/process overhead (approximately 307 ms in 1T and
321 ms in 4T). No completed-depth value is compared across engines.

The qualified runs are aspiration-300ms-1t and aspiration-300ms-4t-retry.
Their pre/post host checks found no process above the 25% CPU threshold;
these snapshots are not continuous host isolation. Earlier attempts are
preserved separately: aspiration-300ms was blocked before searching;
aspiration-300ms-idle completed 320 searches but failed the final load check
for an OS background job; aspiration-300ms-4t completed 160 searches but
failed the final load check for an unrelated build. None were pooled into
the qualified results. No other job was stopped.

All 10,936 PV moves from 1,072 completed searches replayed legally against
the frozen Sekirei rules tool. This includes unqualified timing runs for the
legal-PV check only; it does not qualify their performance. This is a
same-rules check, not an independent oracle. Full manifests, paired outcomes,
raw-row hashes, validation and decisions are in [results.json](results.json).

The candidate remains disabled. A separate 1T material self-play H0=0/H1=10
paired gate is the next strength test. It requires fresh Sekirei-generated
openings, independent of this diagnostic corpus and Claude's hold-out. The
existing diagnostic generator explicitly marks its output diagnostic-only;
changing the seed alone does not create a strength gate. No SPRT or opponent
match was launched, and superiority over YaneuraOu has not been established.
