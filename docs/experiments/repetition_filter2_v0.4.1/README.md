# Two-bit repetition filter (default off)

Source base: `0f015b3a3f467e5e8e6d1400b0ab0df1c5a061c2`.
The `repetition_filter2` Cargo feature is forwarded by the USI package and
records two hash indices in the existing 64-bit search-line Bloom filter.
A clear required bit rules out a line match; a filter hit still uses the
unchanged exact hash scan and perpetual-check adjudication. Three game-history
occurrences bypass this early rejection, as before. Frame size is unchanged.
The compile-time mode prevents a runtime option change from mixing insertion
and query rules. No evaluator, history table or pruning rule is added.

Test-only thread-local counters demonstrate both a saved short-path scan and
a long-path saturation counterexample. They are excluded from all benchmark
executables. Owned-history prefix equality tests cover draws, both colors of
perpetual check, game/line boundaries, siblings and deliberate collisions.
The optimization may lose time through extra bit operations or saturation;
source-level saved scans do not prove an engine speed improvement.

## Prospective development protocol

Commit source and this protocol before native builds or searches. Freeze two
clean standard native release builds at the same commit, compiler, target path
and environment: A without features, B with only `repetition_filter2`. Preserve
both engine hashes, build logs and receipts. No tune, PGO or diagnostic counters.

Use the existing twenty development positions (10 quiet and 10 tactical),
SHA256 `de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Material evaluation, Hash64, MoveOverhead0, SpecTopN0, LazyFlags15,
SearchMode=LazySMP, no book; fresh process per search. This corpus is diagnostic
only and cannot qualify independent playing strength.

1. Fixed depth8, 1T/4T/8T separately, one per-position ABBA repeat (240 searches).
   Require exact 1T bestmove/score/unit/bound/PV/nodes/completed-depth identity.
   Record shared-TT scheduling variation at 4T and 8T without an exact-node
   requirement. Require legal returned PVs in every completed search.
2. Only after correctness, fixed time300ms, 1T/4T/8T, two per-position ABBA
   repeats (480 searches), counters off. Use the existing 25%-CPU pre/post
   host guards. At most two attempts, and attempt2 only if attempt1 blocks
   before any search, separated by at least60s. No retry of a completed or
   interrupted run, process termination or busy-host override.

The 8T development screen requires geometric mean NPS ratio >=1.005 and a
position-bootstrap 95% lower bound >1 (5,000 draws, seed20261015), nonnegative
median completed-depth delta overall and in both corpus categories. 1T/4T
must each retain nonnegative median depth delta and NPS ratio >=0.99. Report
both ABBA directions. Neither speed nor depth may rescue failed identity or
host qualification. Keep every run and unqualified result separately.

This is a development screen, not an independent H1 or a YaneuraOu comparison.
The feature remains default off after this screen. A later independent gate
must pin its own unused corpus and conditions before any default adoption.
The existing independent aspiration queue is not changed.
