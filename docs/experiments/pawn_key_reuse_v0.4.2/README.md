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

## Closed outcome

Source `049ffcd578191c4bf681e9968bec8869cc65b747`, tree
`32cb1874073b8fdf906cb2b4ba07e03f5c0880e0`. Both release engines use this
source and differ only by the default-off feature. Their complete USI replies
are identical; native test-observer symbols are absent. Baseline SHA-256
`f4e4f3195fb45a50ed0a37b75f035a3e309cf4cc27e9b199aa01e46be0aff66e`;
candidate `6ab973ed3fb1713d8d3682f182a034cebd39dd76e2fa86e5ba73aca0f7317285`.

Both core configurations passed 441 tests (nine ignored each). Formatting,
workspace Clippy, feature Clippy, fifteen final protocol tests and eleven
final archive tests passed. Two unsuccessful archive-fixture preparation logs
remain in the frozen inventory; they preceded all native searches.

The A/A control (80 rows) and A/B equality lane (160 rows) passed exact
bestmove/score/PV/nodes/depth identity. All 720 rows and 6,544 PV moves passed
the same-rules legal replay. All worker census, zero/aborted work and raw
transcripts were retained.

The time lane completed its single 480-row attempt. Nine prequiet samples
ended with five consecutive idle snapshots; the formal pre-search guard was
also idle. The post-search guard found a `rustc` process at 99.5% CPU.
Consequently **timing is unqualified**, and no speed claim follows. No
process was interrupted and no search or timing attempt was repeated.

The following values describe the retained unqualified observations only:

| Threads | Median paired wall-NPS difference | Mean-position NPS 95% interval | Median completed-depth difference |
|---|---:|---:|---:|
| 1T | -0.09% | -0.54% to +0.27% | 0.00 ply |
| 4T | +0.86% | -3.42% to +1.11% | 0.00 ply |
| 8T | -0.42% | -0.79% to +3.64% | 0.00 ply |

Both independent speed screens declined advancement. In addition to invalid
timing, neither NPS interval has a positive lower bound and the directional
checks fail. The feature remains **default-off**. No H1 was run, no strength
improvement was established, and YaneuraOu superiority was not measured.
This closes this preregistered observation; its results are not pooled with
other studies or used to justify a new timing attempt.

[Machine-readable results](results.json) binds 84 original artifacts. The
external durable archive contains 89 files, including both binaries, raw
records, failed preparation logs, the full Git source tree/raw commit and a
standard-library-only verifier. Archive manifest SHA-256:
`c5078723bd9b44fa4b515559f41d4019590164efee17fe016f8069813f042f49`.
Archive path:
`~/.local/state/sekirei/search-experiments/pawn-key-reuse-v0.4.2/049ffcd578191c4bf681e9968bec8869cc65b747/`.
Verification passed outside the worktree with `PATH=/nonexistent`, without
Git or reading the original raw paths. The prospective README is preserved
in the source snapshot and input binding. Source-to-final changes after the
measurement are limited to this outcome appendix and `results.json`.
