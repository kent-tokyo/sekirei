# Preserve same-generation TT entries at a two-ply write gap

Prospective single-factor experiment, 2026-10-16, base
`a855d6f1e98ed20e8afb90fbf6a8be8d01365800` (v0.4.2).
Use existing T_TT_KEEP_DEPTH=2 in A and 1 in B, with T_TT_BUCKET=3.
Only a same-hash, same-generation, non-Exact write two plies shallower changes
its eligibility: keep 2 replaces, keep 1 rejects. The production default remains 2.
No new pruning, NNUE training, automatic default adoption or prior-protocol
reopening is included. Independent aspiration/H1 candidates stay unchanged.

Commit this protocol and source before one clean standard native tune build:
Rust 1.97.0, existing target-cpu=native configuration, no PGO/extra compiler
flags, jobs 1, incremental 0 and free space >= 768 MiB. CARGO_TARGET_DIR is this
new worktree's own target directory; do not reuse or modify old caches or
frozen experiment evidence. Save exclusive target/tt-keep-build engine/log/
receipt. Record only filtered unexpected target users for attribution;
never publish all process command lines. Audit native symbols for the private
cfg(test) observer's complete absence.

Audit all source parameter defaults/ranges against the tune USI handshake.
Pin known unrelated experiments OFF, T_SEARCH_V2=1, T_TT_BUCKET=3, LazyFlags=15,
Hash=64, material/no evaluation file, no book, SearchMode=LazySMP, SpecTopN=0,
MoveOverhead=0. Existing non-experiment defaults remain declared defaults.
Validate each row's exact actual options, binary/source/corpus hashes and
budget. All native timing counters are OFF.

Separate actual-activity evidence comes from one isolated cfg(test) owner:
36 unique replacement-boundary cases and 160 search-matrix cases, plus totals
for each 1/2/4/8T and keep 2/1 arm. Require exact successful named Rust test,
boundary keep 2 replace/keep 1 reject at the eligible gap, counter arithmetic,
and positive eligible-gap activity in each search thread/arm. Negative score
fields are valid. Completed depths are retained without a lower bound or
requested-depth equality. These 196 cases are instrumented unit evidence,
not native speed counters. Generic same-hash/rejection aggregates alone cannot
prove actual two-ply-gap eligibility or causal search benefit.

All lanes use the same 20 development positions, 10 quiet/10 tactical,
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Previous local frozen drivers/legal-tool copies are currently unavailable.
Prepare fresh drivers and compile a new same-rules legal tool from this same
clean source, tune feature, compiler/native configuration and dedicated target,
after freezing the new engine. Bind its source, build log and executable SHA
before first search; do not fabricate an old tool hash or regenerate prior
measurements. This is a new protocol, not reopening old evidence.
No hold-out or teacher-derived selection is used.

1. Control: same tune, keep 2/2, depth 8, Threads 1/2/4/8, one ABBA repeat:
   320 rows. Require 20 exact 1T signatures (bestmove, score/unit/bound, PV,
   nodes, completed depth). Record parallel variation. This is a harness
   control, not historic normal-build equivalence.
2. Mechanism: keep 2/1, depth 8, same counts, one ABBA repeat: 320 rows.
   Enable LazyDiagnostics through the existing fixed-depth policy-diagnostic
   wrapper. Retain worker 0 foreign/local/unknown TT hits, worker counters and
   write statistics. This instrumented lane cannot qualify timing. It may
   describe foreign-TT ratios; aggregate changes alone do not establish cause.
   All zero rows/workers are retained. 1T foreign hits are definitionally 0;
   require positive worker 0 foreign-hit aggregates only across all 20 positions
   for each 2/4/8T arm, not every row. The existing equal_depth_overwrites
   counter covers the old direct-table write path and stays 0 in bucket 3;
   it is not evidence that no same-depth overwrite occurred. Committed writes
   retain their existing all-layout accounting; no counter correction is mixed
   into this candidate.
3. Nodes: keep 2/1, 20000 requested nodes, same counts, two ABBA repeats:
   640 rows. Always timing_qualified=false. Retain actual counts, overshoot,
   shortfall, depth 0 and terminal rows without node equality or stop-reason
   inference. Policy changes need not preserve 1T search signatures.
4. Timed: keep 2/1, 300 ms, same counts, two ABBA repeats: 640 rows; counters OFF.
   Schedule regardless of node effects, after complete correctness/PV checks.
   Before attempt 1 only, wait at most 600 seconds for five consecutive available
   idle 25-percent snapshots spaced 10 seconds apart; record every sample.
   Unknown hosts are not idle. Sleep steps <= 30 seconds. Timeout still runs
   attempt 1 through its normal guard. Both available pre/post guards must pass.
   Only a first attempt blocked before any search permits one retry after
   >= 60 seconds, with no new 600-second wait. Never retry observed/interrupted/
   post-invalidated runs, stop another job, override busy checks or run a third.

All completed rows replay every PV before the next lane. If all timed rows
complete, total native observations are 1920. Preserve failures/blocked runs,
all positions/repeats/directions and legal-PV receipts. Never pool candidates
or reopen closed timing protocols.

4T/8T advance independently only from qualified timed evidence: overall
median paired completed-depth delta >= +0.5 ply, quiet/tactical medians >= 0,
both ABBA directional medians >= 0, and strictly positive 95% position-bootstrap
lower bound for mean depth delta. Average each position over both repeats,
5000 resamples of 20 positions, seed 20261016, floor(p*(draws-1)) percentiles.
1T/2T are descriptive controls. NPS cannot rescue a depth failure.

TT policy changes the search tree. A useful-depth failure is not Elo H0,
playing-strength rejection or proof of ineffectiveness. Defaults stay keep 2;
H1 and playing-strength evaluation remain unperformed. This experiment cannot
establish YaneuraOu superiority or replace independent aspiration/H1 work.

```sh
python3 target/tt-keep-build.py --source-ref <full-clean-source>
python3 target/tt-keep-run.py --source-ref <same-source>
python3 target/tt-keep-collect.py --source-ref <same-source>
```

New exclusive outputs use target/tt-keep only. Freeze all new drivers/tests,
source/default/USI/catalog/protocol bytes, logs, executable and copied legal
tool before search. Collector recomputes normalized reports and all hashes;
an existing result is verified without replacement. Exclude growing
orchestration/collection/CI/PR/integration logs from immutable artifacts.

After finalized collection, export a portable durable evidence bundle with
`target/tt-keep-archive.py`. Bind that exporter and its tests before first
search. Retain original JSON bytes and original path aliases unchanged; the
portable index maps aliases to copied hash-verified files. Verify the durable
export before cleaning or archiving any local worktree/cache. This does not
restore unavailable prior local artifacts or regenerate old measurements.
