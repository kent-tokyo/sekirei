# Lazy SMP useful-work diagnostic and first SPRT candidate

Status: diagnostic completed on 2026-10-12; the first candidate is
preregistered for a material-only SPRT. No playing default has changed.

This experiment adds two measurements that were missing from the v0.3.72
work-overlap report:

- a sampled TT hit is `foreign` only when the bounded diagnostic registry can
  identify a different worker as the last worker whose store actually
  committed that entry;
- `discarded root work` is the sum of every non-selected worker's nodes.
  `divergent discarded work` is the subset whose final root move differs from
  the selected result.

The writer registry samples 1/256 TT operations and is allocated only when
`LazyDiagnostics=true`. It is bounded, lock-free, and diagnostic-only. An
unknown writer is kept separate instead of being guessed as local or foreign.
These counters add atomic traffic and are not an NPS baseline.

## Fixed diagnostic

The source base was `origin/main` at `dff8b62`. The measurement used arm64
macOS, material evaluation, `Hash=64`, four threads, 300 ms, two repeats, and
the first six positions from `data/gate/openings_standard.sfen` (SHA-256
`2240cc495e02d50befd45b445f4218d84ae6cdff355db952bdf776f82ad1a1e5`).

| LazyFlags | Selector | Median depth | Bestmove stability | Foreign TT hits | Discarded root work | Divergent discarded work |
|---:|---|---:|---:|---:|---:|---:|
| 15 | deepest, then score | 15.5 | 75.0% | 45.7% | 75.1% | 16.2% |
| 31 | depth/score vote | 14.5 | 83.3% | 45.8% | 74.6% | 31.0% |

The default result shows that shared-TT work is materially reused: nearly half
of sampled hits with a known writer came from another worker. It also shows
that selecting one root result leaves about three quarters of aggregate worker
nodes outside the returned result. Those observations motivate testing the
existing vote selector, but do not prove that it is stronger. In particular,
the vote diagnostic lost one ply of median depth.

## Candidate 1 preregistration

Candidate A changes only `LazyFlags=15` to `LazyFlags=31`, enabling the
existing depth- and score-weighted root vote. Candidate B retains 15. Both
sides use the same v0.3.72-derived binary, material evaluation, `Hash=64`, four
threads, `SearchMode=LazySMP`, the Sekirei-generated material opening corpus,
and colour-reversed games.

The exploratory SPRT is fixed before starting: H0 = 0 Elo, H1 = +10 Elo,
alpha = beta = 0.05, 300 ms per move, and at most 800 games. H1 is a promotion
to an independent confirmation run, H0 is rejection, and reaching the cap is
`INCONCLUSIVE`. Diagnostic percentages are never substituted for the match
verdict, and games from earlier vote experiments are not pooled into this run.

## Reproduction

```bash
python3 scripts/nps_threads.py target/release/sekirei \
  --threads 4 --movetime 300 \
  --positions data/gate/openings_standard.sfen --count 6 --repeats 2 \
  --lazy-diagnostics --option SearchMode=LazySMP --option LazyFlags=15 \
  --json target/lazy-useful-work-default.jsonl

python3 scripts/nps_threads.py target/release/sekirei \
  --threads 4 --movetime 300 \
  --positions data/gate/openings_standard.sfen --count 6 --repeats 2 \
  --lazy-diagnostics --option SearchMode=LazySMP --option LazyFlags=31 \
  --json target/lazy-useful-work-vote.jsonl
```

The 12-search diagnostic is deliberately small. It establishes that the
counters work and supplies a candidate-selection rationale; it is not Elo or
general scaling evidence.
