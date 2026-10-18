# ProbCut SEE lower-bound candidate

Status: **default OFF; speed unmeasured**. This candidate is independent of
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

The [verification summary](results.json) records the final checks. These
material-only correctness checks do not establish NNUE or 8T performance.
No speed value, YaneuraOu comparison or playing-strength gain is claimed.
Native OFF/ON same-source fixed-depth identity and a fresh own-evaluator
8T ABBA study are still required before considering default adoption.
