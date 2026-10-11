# Lazy SMP scaling diagnostic (v0.3.71 base)

This is a bounded material-evaluator diagnostic, not a strength result. It
uses the first six positions from
`docs/experiments/book_coverage_v0.3.68/heldout_openings.sfen`, `UseBook=false`,
`Hash=64`, and the default Lazy SMP flags. The candidate was based on
`651cf561cd2d6b2330976866f7659bfc813d450b`; only opt-in counters and the
measurement tool differed from that revision. Host: arm64 macOS 27.0.1.

## Wall-time lane

Each position was searched twice for 300 ms without counter instrumentation.

| Threads | Median NPS | 1-thread ratio | Median depth | Repeated best-move stability |
|---:|---:|---:|---:|---:|
| 1 | 1,754,262 | 1.00 | 15.0 | 100.0% |
| 2 | 2,796,418 | 1.59 | 15.5 | 83.3% |
| 4 | 4,685,936 | 2.67 | 14.5 | 83.3% |

The four-thread run processed more nodes but did not increase median completed
depth on this small corpus.

## Counter lane

The opt-in counter run searched each position once for 300 ms. Counter-enabled
NPS is not a timing baseline.

| Threads | TT hit rate | Same-hash writes / attempts | Collision overwrites / attempts |
|---:|---:|---:|---:|
| 1 | 35.7% | 39.0% | 0.0001% |
| 2 | 38.2% | 41.3% | 0.0004% |
| 4 | 39.8% | 42.4% | 0.0045% |

Physical slot collisions are too rare here to explain the scaling gap. Shared
TT raises the hit rate modestly, while same-hash rewrites rise only a few
percentage points over the one-thread baseline.

As a causal control, four isolated TTs (the same total hash budget) reached a
median 5.12M NPS versus 4.66M for the counter-enabled shared-table run. A
fixed-depth-10 repeat also showed strong schedule sensitivity: shared-TT
best-move stability was 75.0%, versus 91.7% with isolated tables. These tiny
runs do not establish that isolation is stronger; they show that a later gate
must separate raw contention, shared-table reuse, duplicated work, and result
selection.

## Decision

- Keep the default search unchanged.
- Treat TT slot collisions as a low-priority hypothesis at 1--4 threads.
- Next measure useful shared-TT reuse and duplicated root/subtree work, then
  preregister one worker-diversification or result-selection candidate.
- Measure 8 and 16 threads only on a host with enough physical cores; an
  oversubscribed laptop run would not answer the scaling question.
