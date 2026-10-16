# Root-hint aspiration retry experiment (v0.4.2)

Prospective protocol, before natural-exposure or paired measurements. Baseline:
`cfa03162b6f7da8094ab1c11ae8aaf1933948f77`. This studies a default-off
`ROOT_HINT_RETRY` change within the existing opt-in helper-root-hint policy.
The initial root-hint policy has no independent H1 qualification. A result here
cannot qualify that combined policy for default adoption or establish superiority
to YaneuraOu. No NNUE changes, opening selection or playing-strength claim.

## One factor and frozen inputs

One clean committed source produces one native release `tune` engine and a
same-source legal-move tool. Both arms use that exact engine SHA-256.
A sets `T_ROOT_HINT_RETRY=0`; B sets it to `1`. The harness control sets both
to `0`. Both arms keep `LazyFlags=2063` (default flags 15 plus root-hint bit
2048), shared TT, Hash=64 MiB, material-only evaluation, FV_SCALE=24,
SearchMode=LazySMP, SpecTopN=0 and MoveOverhead=0. All other options retain
verified common defaults. Root hints do not create a channel with one worker;
therefore 1T must be an exact no-op.

Use Rust 1.97.0, the committed `target-cpu=native` configuration, jobs=1,
incremental=0, no PGO and no compiler overrides. Freeze source/tree, build
commands, feature assignment, executable hashes, configuration, test logs,
this prospective document and all measurement/validation helpers before the
first paired measurement. Record their hashes before the natural-exposure test
as well, binding its executable to the clean committed source. Mechanism tests must deterministically exercise retry refresh,
generation/depth rejection and legal-order preservation. Their source/test
proof is distinct from native depth and timing measurements. Existing native
counters do not count retry-refresh branch execution specifically; natural
refresh counts cannot be inferred from publications, applications or stale
nodes. Deterministic cfg(test) mechanism tests provide that branch proof.

Reuse the existing twenty development positions, ten quiet and ten tactical:
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Use its existing category manifest without selection or modification.

## Isolated natural exposure prerequisite

After the source commit and before freezing any native lane, root runs exactly
one isolated cfg(test) diagnostic:
`search::root_hint_retry::tests::natural_retry_hint_exposure_isolated`.
Root builds an optimized cfg(test) executable with `--release --features tune
--lib --no-run`, then executes that exact isolated test once. This executable
contains the private observer and is separate from the native timing engine.
Its observer and test-harness overhead are present; it provides exposure
evidence, not native speed measurements. An inactive result conservatively
declines the screen and does not establish that the policy never activates.
It searches the same twenty development positions once at 300 ms with four
workers and once with eight workers, flag=1, LazyFlags=2063 and Hash=64 MiB.
Its private atomic counts actual retry-order promotions only. It emits forty
per-position counts (all 4T positions followed by all 8T positions) and two
thread aggregates. These forty searches are test diagnostics, separate from
the native 640-row plan; no elapsed-time or speed statistics enter the verdict.
The test observer must be absent from the native executable.

Bind the complete raw test log, exact successful test/footer, forty unique
cases, two checked sums, source/tree, corpus and test-executable SHA-256 in
`target/hint-retry-natural-exposure.json`. Freeze both receipt and log before
any native search, and archive the test executable with their evidence.
Zero promotions is a successful but diagnostically inactive observation, not
a reason to retry. Each thread's native depth screen additionally requires
its own natural-diagnostic aggregate promotions to be greater than zero.
One thread's activity cannot qualify the other. This proves natural exposure
in that diagnostic, not a causal depth benefit or exposure in every native row.

## Four sequential lanes

All lanes use unchanged `paired_search_diagnostics.py`, fresh engine processes
and per-position ABBA order. Output directories must be new. Replay every
recorded PV with unchanged `validate_paired_pv.py` and the frozen legal tool.
Preserve all rows and all workers, including zero work, depth zero, aborted
workers and helper-selected results. The replay is a same-rules check, not an
independent rule oracle.

1. A/A harness control, 1T, depth 12, one repeat: 80 rows. Every ABBA block
   must match bestmove, score, PV, nodes and completed depth exactly.
2. A/B no-op equality, 1T, depth 12, one repeat: 80 rows, the same exact
   signature requirement.
3. A/B policy diagnostics, 4T and 8T, depth 12, one repeat: 160 rows.
   Use `--diagnostics --policy-diagnostic --require-transfer`; validate all
   worker/counter records and extended root-hint records. Parallel signatures
   may differ. This lane is not a timing baseline.
4. A/B native timing, 4T and 8T, 300 ms, two repeats: 320 rows. General
   Lazy/TT/NMP/aspiration diagnostics and latency instrumentation are off.

The maximum is 640 rows. No fixed-node lane, alternative corpus, adaptive
budget, pooled restart or native retry is permitted. Per-search timeout is
120 seconds. Existing root-hint counters and their bookkeeping remain present
whenever root hints are enabled, including the native timing lane. Both arms
retain this same bookkeeping; the native lane is not fully counter-free.
Publications, applications and retry work are descriptive and do not prove
that a hint caused a cutoff or that a refreshed hint caused a depth change.

Before the timing lane, wait at most 600 seconds for five consecutive idle
snapshots ten seconds apart, using the existing host guard. The driver's
before and after guards must also pass. There is one timing attempt at most.
Busy, unknown, failed, incomplete or post-busy observations remain closed and
unqualified; preserve their evidence and do not retry. No other team build or
measurement runs during these lanes. Do not interrupt unrelated processes.

## Independent 4T and 8T development screens

For each ABBA block compute mean-B minus mean-A completed depth separately for
the selected worker and worker 0. Report raw side distributions, all-position
and quiet/tactical block medians, and both ABBA directional medians. Average
the two repeats within each position, then bootstrap those twenty position
clusters 5,000 times with seed 20261016 for the mean's 95% interval. Also
report wall-NPS percentage differences; NPS does not determine advancement.

A thread count advances only when its isolated natural exposure aggregate is
positive, timing is qualified, both 1T lanes passed,
the selected-depth block median is at least +0.5 ply, the selected-depth
bootstrap lower bound is strictly positive, and selected depth has no negative
quiet/tactical or directional median. Worker 0 must have no negative overall,
quiet/tactical or directional depth median. Evaluate 4T and 8T independently;
one cannot qualify the other. A failure is not an Elo H0 conclusion.

All outcomes retain `default_adopted=false`, `independent_h1=false` and
`strength_claim=false`. Any later H1 must qualify the combined root-hint policy
on independently frozen, unopened material-only openings. Current diagnostics
are not such a gate. Root owns a durable archive outside the worktree with all
raw files, source snapshot, engine/legal tool and SHA-256 inventory.
