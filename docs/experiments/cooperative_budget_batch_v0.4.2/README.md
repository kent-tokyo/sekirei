# Cooperative parent-budget batching experiment (v0.4.2)

Prospective protocol, before any candidate search. Baseline:
`cd77f387fdaed25aa3667d7664c104e9681f5cce`. This tests default-off
`COOP_BUDGET_BATCH` inside the existing unqualified cooperative-root prototype.
No NNUE changes, default adoption, independent H1 or playing-strength claim.
This README remains immutable; outcomes belong in a separate `RESULTS.md`.

## One factor and scope

A single clean committed source produces one standard native release engine
with only the `tune` feature, plus a same-source legal-move tool. Both arms
use that exact engine. A sets `T_COOP_BUDGET_BATCH=0`; B sets it to `1`.
The control sets both to 0. Both retain LazyFlags=16399,
T_COOP_MULTI=1 and T_COOP_PERSISTENT=1. Set COOP_PRESTART, COOP_PRESTART_CAP,
COOP_PRESTART_DIAG, COOP_DRAIN, COOP_LATENCY, COOP_PRIORITY, COOP_HELPER_LMR,
ROOT_HINT_RETRY and ASP_VOLATILITY to 0 through their T_ options. Material
only, FV_SCALE=24, Hash=64 MiB, SearchMode=LazySMP, SpecTopN=0,
MoveOverhead=0, all other common defaults verified unchanged.

Batching applies only to eligible direct-parent cooperative budgets with a
time limit and no node cap. Report progress to the parent in 512-node batches;
keep per-tick stop observation and exact final accounting. The unbatched
parent checks its clock every 4096 combined nodes. The batched mode checks the
same parent start/time limit every 512 local nodes and at job end; this changes
the polling cadence, never the configured deadline. Node-capped and untimed paths retain
established behavior. Nested children are not newly batch-enabled; their
existing tick delegation can inherit an already batched parent's behavior.
The measured cooperative root disables YBW and creates no nested Budget.
The non-tune build excludes the added batching fields and branch.

Use Rust 1.97.0, committed target-cpu=native configuration, standard release,
jobs=1, incremental=0, no PGO or compiler overrides. Preserve exact source/tree,
commands, configuration, binaries, successful test receipts and failed logs.
Root verifies unit/integration boundaries: node cap, no time limit, nesting,
normal/panic flush, stop/deadline and exact final parent nodes.

## Natural exposure prerequisite

After committing source, root builds an optimized cfg(test) executable and runs
`search::root_split::tests::budget_batch_natural_exposure_isolated` once.
Use the existing twenty development positions, 300 ms, four and eight workers,
one repeat, candidate=1 and the same prototype configuration. Emit forty
per-position records (all 4T then all 8T) and two aggregates with batched_nodes,
parent_reports and saved_reports. Validate saved_reports=batched_nodes minus
parent_reports and every aggregate against the twenty individual cases.
These are logical parent node-report operations, not a measurement of all
shared atomics. Each thread needs a positive saved_reports aggregate to advance.
Zero is a successful inactive diagnostic, retained without retry.

Freeze a source/tree/input-hash presearch receipt before these forty searches.
Then bind the complete successful isolated raw log, corpus, source/tree and
cfg(test) executable hash in the natural-exposure receipt. The test observer
and its overhead are present only in this separate diagnostic; no duration or
speed statistic from it enters the native verdict. Verify observer absence in
the native timing engine. Natural exposure proves opportunity in that test,
not causal native speed or completed-depth benefit.

## Four native lanes: at most 720 rows

Reuse the existing score-blind twenty development positions, ten quiet and ten
tactical, without selection or modification:
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Do not reuse them as independent H1/hold-out openings.

Use unchanged `paired_search_diagnostics.py`, fresh processes and per-position
ABBA. Freeze source, README, helpers, all input hashes, binary and build receipts
before the first native search; verify the earlier natural presearch binding.
Fresh output directories only, 120-second per-search timeout. Replay every
recorded PV with unchanged `validate_paired_pv.py` and the frozen legal tool.
Preserve zero/depth-zero/aborted work and every raw record.

1. A/A, 1T, depth 8, repeat 1: 80 rows, exact bestmove/score/PV/nodes/depth.
2. A/B, 1T, depth 8, repeat 1: 80 rows, the same exact signature requirement.
3. A/B, 2T/4T/8T, 300 ms, repeat 1, general diagnostics on: 240 rows.
   Validate root/helper node, job, lifecycle and join accounting. This is not
   a timing baseline.
4. A/B, 4T/8T, 300 ms, repeat 2, general diagnostic observers off: 320 rows.
   Existing intrinsic cooperative accounting remains present equally in both
   arms; do not describe this as removal of all counter bookkeeping.

The cooperative path exposes one selected root result, worker 0. Its completed
depth and selected depth are the same observation; helpers are root tasks,
not independent completed iterations. Verify the one-root worker census and
separate helper accounting without inventing helper completion depths.

Before the native timing lane, observe at most 600 seconds for five consecutive
idle snapshots ten seconds apart using the existing host guard. Both driver
pre/post guards must pass. At most one timed attempt. Busy/unknown, failed,
incomplete and post-busy evidence remains closed and unqualified. No retries,
pooling, budget changes or interruption of unrelated jobs. No concurrent team
builds or measurements. Keep diagnostic results separate from native timing.

## Independent 4T and 8T screens

Each ABBA block uses mean-B minus mean-A completed depth. Report overall and
quiet/tactical block medians, both ABBA directional medians, raw distributions,
and selected/worker-0 depth with their common source stated. Average the two
repeats within each position; bootstrap twenty position clusters 5,000 times,
seed 20261017, for the mean's 95% interval. NPS is descriptive.

A thread advances only when natural exposure is active, timing is qualified,
both 1T prerequisites passed, depth median is at least +0.5 ply, the depth
bootstrap lower bound is strictly positive, and no overall, quiet/tactical or
directional selected/worker-0 median regresses. Independently require:

- block median elapsed mean-B/mean-A ratio <=1.005;
- upper 95% bootstrap bound of the twenty-position mean elapsed ratio <=1.01;
- raw B elapsed p95 <= raw A elapsed p95 +5 ms.

Use nearest-rank p95 (`ceil(0.95*n)` in ascending order). Report all raw elapsed
values, max/p95, paired ratios and directional ratios. These fixed gates prevent
qualifying depth gained by simply searching longer. Failure from scheduling
noise remains a closed result; do not retry. Evaluate 4T and 8T independently.
Failure is not Elo H0. A passed screen still leaves the combined cooperative
policy unqualified: default_adopted=false, independent_h1=false and
strength_claim=false. Root archives all raw evidence, source and hashes outside
the worktree before integration.
