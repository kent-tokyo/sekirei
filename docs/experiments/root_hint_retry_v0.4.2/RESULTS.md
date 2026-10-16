# Root-hint retry result: decline at 4T and 8T

`ROOT_HINT_RETRY` remains **0 by default**. The one-shot development screen did
not improve completed depth enough to advance at either thread count. No H1,
default adoption, NNUE change or YaneuraOu superiority claim follows.

The [prospective protocol](README.md) is preserved byte-for-byte as a frozen
input. This outcome and [recomputed raw inventory](results.json) are separate
files; the measured Rust source is unchanged.

## Frozen source and checks

- Baseline: `cfa03162b6f7da8094ab1c11ae8aaf1933948f77`.
- Measured source: `188083e40539ab381d3f43e737aa6b7bd14108b9`.
- Source tree: `9007bf7f4a9929df6f1b01157ca1d4b7c4828f93`.
- Same native tune engine on both arms, SHA-256
  `2ebda3e448929bfc17268d67df8b3d35ddd45cd5bffea6881b039960b6b1c6e2`.
- Both arms use material evaluation, shared Hash=64 MiB and LazyFlags=2063.
  Only `T_ROOT_HINT_RETRY=0/1` differs; existing root hints are opt-in too.
- Default core: 448 passed, 9 ignored. Tune core: 447 passed, 24 ignored.
  The isolated mechanism test passed 13 boundary cases plus exact MultiPV and
  fixed-root comparisons. Workspace/tune Clippy and formatting passed.
- Protocol 21 tests, archive 11 tests and integration audit 16 tests passed.
  A preparatory unoptimized full test was stopped and retained as incomplete;
  it is not a successful check or a performance observation.

The optimized cfg(test) exposure diagnostic recorded 17 actual retry-order
promotions at 4T and 17 at 8T, across all twenty positions once per thread
count. Its forty searches include observer overhead and are separate from
native timing. These counts prove exposure in that diagnostic, not a causal
depth gain or activity in every native row. The native engine contains neither
private test observer.

## All 640 native rows retained

A/A 1T control and A/B 1T no-op equality each completed 80 rows. All forty ABBA
blocks matched bestmove, score, PV, nodes and depth exactly. The 4T/8T fixed-depth
policy lane retained 160 rows; the 300 ms lane retained 320 rows. All 4,788 PV
moves passed the frozen same-rules legal-move replay, with zero violations.
This is not an independent rule oracle or a mate proof.

The prequiet guard passed on sample 35, and the driver's before/after guards
passed. Timing is qualified under the preregistered host-guard contract.
Existing root-hint counters remain present in both timed arms; this lane is
not fully counter-free. No row, worker or position was dropped.

| 300 ms, B minus A | 4T | 8T |
|---|---:|---:|
| Selected completed-depth block median, ply | -0.25 | 0 |
| Selected depth mean over position clusters, ply | -0.0625 | -0.0625 |
| Selected depth mean 95% bootstrap interval | [-0.2625, 0.1375] | [-0.2375, 0.125] |
| Worker 0 completed-depth block median, ply | -0.25 | 0 |
| Worker 0 mean over position clusters, ply | -0.10 | -0.125 |
| Wall-NPS block median change | -0.312% | -0.394% |
| Independent development screen | decline | decline |

Both screens miss the +0.5-ply median requirement, have depth intervals crossing
zero, and contain category or worker-0 regressions. The observations do not
establish an Elo loss or an Elo H0 decision. NPS did not decide advancement.
The study is closed; no native retries, pooled restart or SPRT were performed.

## Durable evidence

All 106 original inputs and outputs, both executables, the optimized exposure
test executable, full source tar/raw Git commit, results and standalone
standard-library verifier are stored outside the worktree:

`~/.local/state/sekirei/search-experiments/root-hint-retry-v0.4.2/188083e40539ab381d3f43e737aa6b7bd14108b9/`

The 111-file archive manifest SHA-256 is
`44d6bc48c172463beee5269b50657971c38d882704a0fb465b915dd2caa31898`.
Offline verification passed without Git or a source checkout. It verifies
file hashes and reconstructs the Git tree/commit; it does not independently
recompute the statistics. The frozen collector recomputed every validated
lane from the original parsed rows. Full raw USI transcripts are not saved by
this driver.

An archive copy failed during disk exhaustion. Verified-unused release compiler
intermediates and the incomplete duplicate were removed, and the copy succeeded
from the same unchanged originals. No search was repeated. Recovery and final
integration checks are retained separately from the closed native study.
