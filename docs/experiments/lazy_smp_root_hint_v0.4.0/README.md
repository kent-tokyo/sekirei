# Lazy SMP helper-root-hint experiment (v0.4.0 base)

Status: diagnostic implementation complete; independent strength gate not yet
opened. The playing default remains unchanged.

## Hypothesis

`LazyFlags` bit 2048 lets a helper publish the move and depth from a fully
completed exact root iteration. Worker 0 may try a same-or-deeper hint first
on its next iteration. Scores, bounds, and PVs are not shared through this
channel.

The one-factor candidate is `LazyFlags=2063`; the baseline is
`LazyFlags=15`. The experiment changes root ordering only. Evaluation,
pruning constants, worker count, TT sharing, time management, and final result
selection stay fixed.

The implementation commit is `a01bc23`; invocation-bound counters were added
by `845e9aa`, and the fail-closed measurement contract by `58347e2`. A later
strength run must record its final built commit and binary SHA-256 rather than
assuming these source commits identify the executable.

## Diagnostic before the gate

The diagnostic must use material evaluation, four threads, `Hash=64`, no
book, a fixed wall-time budget, and repeated searches of the same frozen
positions. Run it on an otherwise idle host:

```bash
python3 scripts/nps_threads.py target/release/sekirei \
  --threads 4 --movetime 300 --repeats 2 \
  --positions /path/to/frozen-root-hint-diagnostic.sfen \
  --count 20 --option SearchMode=LazySMP --option LazyFlags=2063 \
  --require-root-hint-diagnostics \
  --json target/lazy-root-hint-diagnostic.jsonl
```

Every JSONL row records the binary SHA-256, source and effective position-set
SHA-256, evaluator, options, budget, and host. The strict lane rejects a
missing, duplicate, malformed, overflowing, or inconsistent root-hint line.
It reports:

- helper publications accepted as slot updates;
- same-generation, same-or-deeper reads by worker 0, before the final root
  list is checked;
- reads that actually changed the final legal root order;
- searches with at least one publication or application;
- reads per publication and applications per eligible read.

Zero publications or zero eligible reads are reported as unavailable ratios,
not as zero-percent effects. Counter-enabled runs are diagnostics and must not
be used as timing baselines.

## Advancement rule

Do not open the strength gate unless all of these hold:

1. every planned search has one valid root-hint record;
2. at least one frozen position has an actual root-order application;
3. the candidate has no material completed-depth regression against a
   counter-free `LazyFlags=15` run under the same position order;
4. fixed-budget legality, mate safety, PV, and hash regressions remain zero;
5. the currently running depth-consensus independent gate has reached a
   terminal result and its processes are stopped.

The root-hint strength test must then use an unopened material-only opening
set, colour reversal, four threads, `Hash=64`, the same per-move wall-time for
both sides, paired trinomial SPRT with H0=0 Elo, H1=+10 Elo, and
alpha=beta=0.05. Freeze the binary, opening file, options, and their SHA-256
values before the first game. Do not pool diagnostics, exploratory games, the
depth-consensus gate, or a restarted run.

H1 is required for adoption. H0 rejects the candidate. A capped run is
`INCONCLUSIVE` and does not change the default.
