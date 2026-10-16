# Existing V2 fail-high score blend

Prospective single-factor experiment, 2026-10-16, base
`7a23c91bcfe0fb01284e02737946afffab97bc82` (v0.4.2).
Compare the existing T_V2_FH_BLEND=0/1 only. Its production default stays0;
no new search policy, NNUE training or default adoption is included. A separate
cfg(test) activity observer verifies actual policy execution in an isolated
libtest process; native measurements contain no observer counters. Retain
the exact named passing test, all160 unique1/2/4/8T ×20positions ×OFF/ON
rows, zero OFF counters, positive ON applied/changed totals at each count,
matching raw-case/printed-total arithmetic and both mate-root guard rows.
The parsed mate rows expose score/depth equality only; full tuple/PV equality
comes from the exact passing Rust assertion. No worker0 depth or decisive
tailguard rejection is inferred from unexported fields or the mate bypass. This is
separate from the queued aspiration/H1 protocols, which stay unchanged.

Commit source and this prospective protocol before native build/search. Use
one clean standard native tune release build, Rust1.97.0, existing
`.cargo/config.toml` target-cpu=native, no PGO/compiler overrides,
CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=1 and at least768MiB free. Reuse only the
build cache at `/Users/k_tanabe/.codex/worktrees/lazy-smp-useful-depth/sekirei/target`;
do not modify its frozen engine/evidence copies. Save a new exclusive
`target/fh-blend-build/{engine,receipt.json,build.log}`. Audit native symbols
for the test observer. This is one tune binary in every arm; the control lane
is harness identity, not historical normal-build default equivalence.

Audit every declared search-parameter default/range against the frozen tune
USI handshake, save defaults.json and bind source/inputs/executables/scripts
before the first search. Explicit common controls retain known defaults:
LazyFlags15, T_SEARCH_V2=1, MoveOverhead0 and disabled unrelated experimental
flags. Established defaults such as V2_STAGE_GEN/V2_ROOT/V2_KEEP/CAPT_LMR and
NNUE_FINNY remain as declared. Hash64, material evaluation, no book,
SearchModeLazySMP and SpecTopN0 are fixed. Lazy/NMP/aspiration/cooperative
observers remainOFF. Validate each row's actual effective options exactly,
including FV_SCALE24/ThreadsN/Hash64/SearchMode/SpecTopN and assigned factor;
reject duplicates, unknown options or arm mismatches.

All lanes retain the same20 development positions,10quiet/10tactical,
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Copy the prior frozen same-rules legal tool exclusively into this experiment,
SHA-256 `dfbe08d01ced743c0bbd409fe0ca816599d53daa4ef6dcbaa447e381c8d9dc00`.
No hold-out data or Suisho-derived selection is used.

1. Control: same frozen tune binary, A0/B0, depth8, Threads1/2/4/8,
   one per-position ABBA repeat:320rows. Require all20 1T blocks to match
   bestmove/score/unit/bound/PV/nodes/completed depth exactly. Parallel within-
   arm variation is recorded. This is a harness control only.
2. Nodes: A0/B1,20000 requested nodes, Threads1/2/4/8, two ABBA repeats:
   640rows. Always timing_qualified=false, including idle hosts. Retain
   actual node counts, overshoot/shortfall and depth0/terminal rows without
   exact-node equality, exclusion or inferred abort reason. No 1T identity
   is required because the search policy changes. This is a work diagnostic,
   not NPS, strength or adoption evidence.
3. Timed: A0/B1,300ms, Threads1/2/4/8, two ABBA repeats:640rows. Schedule
   regardless of the node-effect result; only complete correctness/PV
   prerequisites gate it. Before the first timed attempt only, wait at most
   600seconds for five consecutive available idle25-percent snapshots,
   spaced10seconds apart; unknown hosts are not idle. Save every sample in
   a new JSON receipt. Sleeps stay<=30seconds. On timeout still perform
   attempt1 through its existing guard; no second600-second wait is allowed.
   Both available pre/post host guards must be idle
   at the25-percent busy-process threshold. At most two attempts: only a
   first attempt blocked before any search permits one retry after at
   least60seconds. Never retry completed/interrupted/post-invalidated runs,
   use --allow-busy, stop another job or run a third attempt. A permitted
   completed retry is judged by its own guards. Retain blocked/negative
   evidence and all positions/directions.

Every completed lane must replay every PV legally before proceeding. Root
cooperative/prestart/CAP records must be absent in every arm. No prior timing
protocol is reopened. One fixed-depth and one node run only; failed/incomplete
prerequisites stop later searches without overwriting evidence.

4T and8T qualify independently only from timed evidence: overall median
paired completed-depth delta>=+0.5ply, quiet/tactical medians>=0, **both
A-first and B-first directional medians>=0**, and strictly positive95%
position-bootstrap lower bound for mean depth delta. Average per-position
ABBA deltas over both repeats, resample20positions5000times, seed20261016,
percentiles at floor(p*(draws-1)).1T/2T are observations only. NPS cannot rescue
a depth failure. Even a positive development screen leaves defaultsOFF
pending independentH1 and establishes neither Elo nor YaneuraOu superiority.
FH_BLEND changes the search tree and potentially search accuracy. Failure of
this useful-depth screen rejects only advancement on the present4T/8T depth
criterion; it is not an Elo H0/rejection or proof that the policy is ineffective.
FH playing-strength evaluation remains unperformed, and this candidate does
not replace the independent aspiration/H1 candidate.

```sh
python3 target/fh-blend-build.py --source-ref <full-clean-source>
python3 target/fh-blend-run.py --source-ref <same-source>
python3 target/fh-blend-collect.py --source-ref <same-source>
```

New outputs use only the target/fh-blend prefix. Freeze all drivers/tests,
finalized test/build logs, corpus/categories, defaults, protocol, USI handshake
and copied legal tool before search. Collector recomputes normalized analyses
and all hashes, saves exclusive results.json, and only verifies an existing
output without replacing it. Exclude growing orchestration/collection/CI/PR/
integration logs from immutable artifacts. Preserve failed diagnostics apart
from completed evidence; do not pool different candidates or corpora.

## Recorded outcome

Measured source: `8843fd965d40a272fef2442b1f37f66f7f66a09d`.
The production guard/formula and all parameter defaults are unchanged; the
added observer exists only under `cfg(test)`. The frozen native tune engine
SHA-256 is `9a96657dee46d36566c9e65095db73647f0085a358f9e04ca0b67e5a2cff5b9b`.
Compiler, shared-cache attribution and native observer-symbol audits passed.

Core tests:435 passed,9 ignored. The exact isolated tune test passed all160
matrix cases and2 mate cases. Its real-score changes were2016/3192/7040/12230
at1/2/4/8T respectively; every OFF counter was zero. These are instrumented
libtest observations, not native performance counters. Workspace/tune Clippy,
format checks,92 measurement-tool tests and the final14 driver tests passed.
Earlier9/14-test logs remain preserved separately, not counted as additional
unique final tests.

Control320, node640 and timed640 searches completed. All1600 rows and12189
PV moves passed legal replay. The1T OFF/OFF control matched exactly in all20
positions. [results.json](results.json) binds78 artifacts and recomputes actual
activity, every analysis, effective options, inputs and raw-result hashes.

| Threads | Node depth delta median | Node mean | Position-bootstrap95% interval |
|---|---:|---:|---:|
|4T|0.0|0.0|[-0.2,+0.2125]|
|8T|0.0|+0.05|[-0.15,+0.25]|

The300ms attempt completed after17 prequiet samples over160.7seconds and
passed its idle pre-guard. Its post-guard detected other CPU-active processes,
so **timing_qualified=false**. The following values are retained unqualified
observations; their intervals do not establish a timing/depth effect.

| Threads | Raw depth delta median | Raw mean | Raw position-bootstrap95% interval | Quiet / tactical median | A-first / B-first median |
|---|---:|---:|---:|---:|---:|
|4T|0.0|-0.075|[-0.2625,+0.1375]|0.0 / -0.5|0.0 /0.0|
|8T|-0.25|-0.225|[-0.3875,-0.0625]|0.0 / -0.5|0.0 /0.0|

Neither thread count advances under the prospective development rule.
A completed attempt is not retryable, including post-invalidated timing;
there was no second attempt and this measurement protocol is closed.
Default staysOFF. No FH playing-strength match or independentH1 was run;
this is neither an Elo rejection nor evidence of YaneuraOu superiority.
The existing independent aspiration/H1 candidate remains queued unchanged.
