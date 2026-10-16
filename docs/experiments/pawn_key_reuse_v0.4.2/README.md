# Pawn-key reuse experiment (v0.4.2)

Prospective protocol, before any native candidate search. Baseline:
`feeb9bde52619d0ecbf0bc2b93ac5067eeb46220`. No NNUE changes, search-policy
changes, additional history tables, runtime counters, or default adoption.

## Single change

The default-off Cargo feature `pawn_key_reuse` derives the V2 parent pawn-history
bucket from the already computed correction key. The correction key includes
14 pawn-hash bits and a side-to-move offset; masking the offset and dropping
five bits reproduces the existing nine-bit pawn-history bucket. In-check nodes
have no correction key and retain the existing board calculation. The reuse
occurs only after child/ProbCut board state has been restored. Other call sites
and the default build retain their existing calculation.

Unit tests must check both sides, all correction buckets, upper/lower bitboard
halves, pawn push/capture/promotion/drop/undo, legal play, and reuse/fallback.
Semantic preservation and measured speed are separate from playing strength.

## Frozen builds and observations

One clean committed source produces two standard native release engines:
A with no features; B with only `pawn_key_reuse`. Rust 1.97.0, the committed
`target-cpu=native` Cargo configuration, jobs=1, incremental=0, no PGO and no
compiler overrides. A same-source legal-move tool checks every recorded PV.
Build commands, source/tree, feature assignment, hashes and test logs are
frozen before the first native search. No other team builds run during search.

Use the existing 20-position development corpus
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`,
with its existing ten quiet and ten tactical categories. No hold-out or
external-evaluator selection. Material-only evaluation, FV_SCALE=24,
Hash=64 MiB, SearchMode=LazySMP, SpecTopN=0, LazyFlags=15,
MoveOverhead=0; every other option retains its verified common default.
No tuning feature, opt-in policy, diagnostics or latency instrumentation.

All searches use unchanged `paired_search_diagnostics.py` and fresh processes
in per-position ABBA order. Keep every row, including zero/aborted worker work.
Native basic worker census is permitted; optional diagnostic counters are off.

1. Harness control: A/A, depth 12, 1T, one repeat (80 rows).
2. Equality: A/B, depth 12, 1T, two repeats (160 rows). Every block must match
   bestmove, score, PV, nodes and completed depth exactly.
3. Time: A/B, 300 ms, 1T/4T/8T, two repeats (480 rows). Qualify each thread
   count independently. Parallel node counts and ordering need not match.

Before the time lane, observe at most 600 seconds for five consecutive idle
snapshots ten seconds apart. The driver's before/after host guards must also
pass. There is exactly one time attempt; busy, failed, incomplete or post-busy
runs remain closed and unqualified. No retries, pooling or budget changes.
The 120-second per-search process deadline remains unchanged.

## Development decision (not H1)

Compute each block's mean-B/mean-A wall-NPS percentage and mean-B minus mean-A
completed depth. Average repeats within each position; use 5,000 bootstrap
samples over the twenty positions, seed 20261016, for the mean's 95% interval.
Compute overall, category and directional medians over all retained ABBA
blocks (forty per thread in the time lane). Report raw data and 1T/4T/8T
separately. Depth-only lanes do not determine the speed verdict.

A 4T or 8T speed screen advances only with qualified timing, exact 1T equality,
median paired NPS improvement >=0.5%, a strictly positive bootstrap lower
bound, positive NPS medians in both ABBA directions, and nonnegative completed
depth medians overall, in both categories and in both directions. Failure is
not an Elo H0 verdict. No default change or strength claim follows, even if a
speed screen passes. Independent H1 and pinned equal-condition YaneuraOu
qualification are separate work.

Freeze drivers and inputs before search. Retain failed logs. Publish all raw
records, both binaries, source snapshot, legal replay and hashes in a durable
archive outside the worktree, then integrate concise results through a PR.
