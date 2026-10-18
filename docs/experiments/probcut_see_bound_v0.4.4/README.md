# ProbCut SEE lower-bound candidate

Status: **default OFF; own 8T speed regression confirmed**. This candidate is independent of
Finny, PGO and the unsuccessful `V2_SEE_BOUND_CACHE` experiment. It does not
change that flag or any pruning threshold.

ProbCut currently computes an exact static exchange evaluation (SEE) for
its move filter. When its comparison threshold is non-positive, an existing
capture-up proof can establish that SEE is at least zero. The candidate
skips the exact calculation only in that case. Positive thresholds, moves
without the proof and disabled builds keep the original exact comparison.
Both the proof and exact calculation are lazy; move entries and ordering
are unchanged.

Enable with the optional Cargo feature `probcut_see_bound` in `sekirei` or
`sekirei-core`. A separate `tune` build can use `V2_PC_SEE_BOUND=1`; its
recorded default is zero. The feature forces ON independently of the tune
parameter and is not in any default feature list.

## Correctness evidence

Focused tests cover threshold boundaries, one-shot closure invocation,
legal captures including promotions and zero exchanges, losing captures,
quiet promotions and drops. They compare against exact SEE and check board
restoration. The isolated material test runs four fixed positions at depths
6 and 8, OFF then ON with fresh 4 MiB TTs. All 16 searches match bestmove,
score, PV, nodes, depth and bounds exactly; every PV is replayed as legal.
It asserts that the real ProbCut comparison is reached and an exact SEE
call is actually skipped. Test-only thread-local observations are absent
from production builds.

Reproduce the identity check in its own process, without the native feature:

```bash
cargo test -p sekirei-core --features tune --lib \
  probcut_see_bound_preserves_material_search_and_reaches_comparator \
  -- --ignored --nocapture --test-threads=1
```

## Native 8T study (own evaluator)

The candidate was **3.36% slower** than OFF: ratio `0.9663897789`, bootstrap
95% interval `0.9570302666–0.9770042304`. Default adoption is rejected for this
workload. Skipping exact SEE is not itself evidence of a speed gain.

Native OFF/ON use clean source `1cc0fe4d`, all 149 source inputs, separate
targets, and no PGO, tune or instrumentation. A disk interruption after OFF
completed was recovered by building only ON with the original recipe; the
original failed manifest and OFF bytes remain bound and unchanged. The
recovery passed eight pure tests and independent review.

Fresh [32 positions](own-speed32.sfen), seed `20261018101`, exclude 265 known
states across nine files. Historical own-r3 HalfKP, FV24 is read-only.
Native 1T depth-12 ABBA gives 128 exact signatures and legal PVs. The 8T study
uses Hash64 MiB, RAYON8, 1000ms and fresh processes in ABBA order. All 32
positions completed: 128 accepted / 164 actual calls, nine host-rejected
quartets and 30 eligible positions. Positions 3 and 16 were excluded by the
preregistered whole-position 700ms minimum. Every actual PV, evaluator load,
raw hash and all 767 input bindings passed independent verification; the
statistics were independently recomputed.

Before candidate speed data, host-only whole-quartet attempts changed from
three to eight. CPU/disk thresholds, quiet checks, full-interval sampling,
minimum 24 eligible positions and bootstrap5000 were unchanged. The first
eligible quartet is used, with no partial statistics or old capture pooling.
The schedule passed 31 pure tests and independent review.

The [verification summary](results.json) binds the receipts. This result does
not establish playing strength or general hardware performance. No external
reference weights were used for this candidate/default decision. A separate
final native comparison will use the OFF control under the rule fixed before
the own study completed.
