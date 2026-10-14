# Borrow completed root moves for V2 learning

This removes one growing move vector per ordinary root pass. The loop visits
`ordered` without skips, so completed moves are exactly its prefix. The prefix
length advances only after undo and the abort check. Existing learning guards,
best-move exclusion, 32-alternative limit, root order and pruning stay intact.
Cooperative root traversal is unchanged. No parameter or NNUE change is made.
This is a behavior-preservation check, not a playing-strength candidate or claim.

## Prospective protocol

Commit this protocol and source before building or searching. Freeze a clean
standard native release tune executable, compiler/config/environment and hash.
Baseline is the frozen f831b6b tune build, SHA-256
`e21805bb94bd173490a1aea66bfa5828a73c6b5d22eb3c9f67a1e5aafdea0cb9`.
Its core/USI source matches parent main d24bc90; verify this before measurement.
Keep its original build receipt; do not present it as a newly built main binary.

Use the existing twenty development positions, ten quiet and ten tactical:
`../lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Material evaluation, no book, Hash 64, MoveOverhead 0, SpecTopN 0, LazyFlags 15,
SearchMode=LazySMP and all tune defaults are identical in both arms. Fresh
processes isolate each search. No optional diagnostic counters are enabled.

1. Fixed depth 8, 1T and 4T separately, one per-position ABBA repeat: 160 searches.
   Require exact 1T bestmove, score/unit/bound, PV, nodes and completed depth in
   all 20 blocks. Record 4T within-arm variation separately; shared-TT scheduling
   does not justify a parallel exact-node claim or illegal PVs. Replay all PVs.
2. Only after correctness passes, counter-free 300 ms at 1T/4T, two ABBA repeats:
   320 searches. Keep quiet/tactical and A-first/B-first results separate.
   Require the existing 25%-CPU pre/post host guard. At most two attempts are
   allowed, and attempt 2 only if attempt 1 blocks before any search; wait at
   least 60 seconds. Retain blocked manifests separately. Never repeat a completed
   or interrupted run, stop another process, or bypass the host guard.

Preserve fixed-depth identity and report all timings, including unqualified
runs. Timing qualification is not a strength gate. No NPS or depth improvement
is claimed from an invalidated run, and no independent ASP candidate is replaced.
The optimization removes allocation/copies by construction; the overall gain
may be too small to resolve. Adoption of a changed search policy still requires
independent H1; this change must keep the search behavior unchanged.

## Observations

The clean native tune build at 30a8ab0 has SHA-256
`5f98a4626be7d6d7e95bd90acd99aa2fc7b390d6a56c68322723f0059117f536`.
[results.json](results.json) binds the protocol, build receipts and raw evidence.
Baseline core/USI source equality with parent main was verified before building.

All 20 one-worker ABBA blocks (80 searches) matched bestmove, score/unit/bound,
PV, nodes and completed depth. The separate 80 four-worker searches had
within-arm nondeterminism in all 20 blocks and no exact cross-arm blocks; no
parallel node-identity claim follows. All 1,050 PV moves across 160 searches
replayed legally through the frozen same-rules tool, with zero violations.
This is not an independent rule oracle or mate-score proof.

Both permitted 300 ms timing attempts were blocked by unrelated compiler load
before any search. The second began more than 60 seconds after the first.
Neither attempt produced rows; no busy-host override, process interruption,
pooled timing or third attempt was used. Overall NPS and depth improvement
remain unmeasured. Removing the growing vector is a source-level allocation
reduction, not proof of a measurable engine speed or playing-strength gain.

Core 361 tests passed with 4 ignored, including existing root fail-high and
abort/TT regressions. Formatting and workspace Clippy passed. The independent
aspiration candidate remains unchanged; no NNUE or parameter default changed.
