# Lazy SMP useful-depth diagnostics (v0.4.0 base)

Status: implementation and fresh diagnostic corpus prepared. No default change
or playing-strength claim follows from these diagnostics. A legacy
independent depth-consensus gate was still running when this work began; it
must reach a terminal result before a new strength gate is opened.

## Frozen input and executable lanes

`diagnostic.sfen` contains 20 new positions: ten quiet and ten tactical,
alternating by category. SHA-256:
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
`diagnostic.manifest.json` records every trajectory, replay move, rule fact,
seed, generator hash, and legal-move/rule-probe executable hash. Generation
starts from the initial board with seed `2026101401`, selects legal moves
without scores, and samples at 24 or more plies. The set spans 12 trajectories
and both sides equally, with at most one position per category per trajectory. No training corpus, NNUE,
external evaluator, or prior strength opening is input to generation. This
corpus is diagnostic-only and must never become a subsequent independent gate.
It is deliberately small and its random trajectories do not establish a
representative playing-position distribution.

Reproduce into a new filename with:

```bash
python3 -B scripts/generate_search_diagnostic.py \
  --legal-moves /path/to/frozen/sekirei-legal-moves \
  --rule-probe /path/to/frozen/sekirei-rule-probe \
  --output /path/to/new-diagnostic.sfen --count-per-category 10 --trajectories 48
```

The initial dirty source snapshot is a separate regression baseline. It already
contains deferred losing-tactical sorting and helper root LMR. Do not label
this snapshot as the pre-change sort baseline. Comparisons of new tuning
switches use one `--features tune` executable for both sides. That isolates
runtime parameters and avoids comparing builds with different feature sets.
The sort baseline is `T_V2_EAGER_LOSING_SORT=1` in the same tune executable.

Every paired run copies both executable streams and the complete input into a
new output directory, hashes those copied streams, and records effective
positions, explicit options, material-only evaluator, budget, host, and start
time. A supplied source label is recorded as unverified: an executable hash
alone does not prove which source built it. Preserve build logs and dirty-source
snapshots alongside the runner's manifest.

## Order and measurements

`scripts/paired_search_diagnostics.py` runs **A, B, B, A on each position**
before advancing to the next position. Each search starts a fresh process and
uses `Hash=64`, `MoveOverhead=0`, no book, no speculative top-N, and material
evaluation. The paired harness explicitly defaults MoveOverhead to zero so
`go movetime 300` provides a 300ms search window. The legacy nps script keeps
its historical engine-default behavior unless this option is supplied. Searches
never run concurrently. This avoids treating two whole-corpus passes as paired
ABBA and avoids TT/history contamination across observations.

Counter-free timing and counter-enabled diagnostic runs are separate output
directories. The runner flushes each raw row immediately, marks interrupted or
invalid runs `incomplete`, rejects incomplete ABBA blocks, and cannot append a
restarted run to an existing directory. No incomplete run advances a candidate.
A USI response timeout terminates and reaps the engine. Read-only process
snapshots record host activity before and after the run. Counter-free timing
refuses a busy or unobservable host by default; `--allow-busy` preserves an
explicitly unqualified exploratory run. External processes are never stopped.
A process consuming at least 25% CPU makes timing unqualified. These snapshots
cannot exclude all transient load between observations; also inspect other
active jobs before treating a speed difference as evidence.

The summary reports position-paired selected depth, wall NPS, elapsed time,
nodes, nonselected worker nodes and helper-only nonselected nodes, worker-0
completed depth and foreign TT hits/rate, deepest-helper depth, helper completed
iterations, and retired helpers where emitted. Medians are over position-block
paired deltas/ratios; they are not ratios of separately pooled medians. TT-origin
rates exclude unknown writers and remain sampled diagnostics.

## Deferred losing-tactical sort

Run fixed-depth identity first, on both one and four workers:

```bash
python3 -B scripts/paired_search_diagnostics.py \
  --baseline target/release/sekirei --candidate target/release/sekirei \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --threads 1,4 --depth 10 --repeats 2 --option LazyFlags=15 \
  --a-option T_V2_EAGER_LOSING_SORT=1 --b-option T_V2_EAGER_LOSING_SORT=0 \
  --output target/useful-depth/sort-identity
```

One worker must have exactly matching bestmove, score with unit and bound, PV,
nodes, and completed depth across all four observations. A mismatch returns a
nonzero exit status. Shared-TT four-worker search is nondeterministic even with
identical code: report observed identities and within-side variation rather
than claiming node-identical four-worker behavior. Investigate differences in
score or legal PV; SMP nondeterminism is not a correctness waiver.

After identity passes, repeat without counters at `--movetime 300`, with
`--threads 1,4` and enough repeats to assess wall NPS. Sort is a speed change
only after the exact one-worker lane passes; any speed claim requires the timed
measurements, not reduced nodes or an expectation from skipped sorting.

## Helper hints, LMR, and retirement

Use the same paired runner separately for each one-bit candidate:

| Candidate | A flags | B flags |
|---|---:|---:|
| helper root hint | 15 | 2063 |
| helper root LMR | 15 | 4111 |
| dominated helper retirement | 15 | 8207 |

Start each factor with `--threads 1,2,4,8 --movetime 300 --repeats 2` for
counter-free depth/timing. Retirement is intentionally inactive below three
workers. Use a separate run with `--diagnostics` for useful-work counters;
for hint-transfer validation add `--require-transfer --threads 2,4,8`.
Do not combine the candidate bits. Eight workers exceeding the host's physical
cores are an oversubscription diagnostic, not a scaling result.

A hint diagnostic records accepted publications, eligible reads, actual legal
root-order changes, first-move beta cuts on the first hinted attempt,
aspiration retries and their nodes, stale observations, depth gaps, and
invocation-generation gaps. `stale_research_nodes` attributes subsequent
retry nodes to an observed superseded hint; neither this nor `research_nodes`
proves the hint caused extra work. Generation gaps must remain zero. Pool
counts before rates, and represent absent opportunities as unavailable ratios.

Root LMR must preserve selected completed depth, increase helper completed
iterations, reduce nonselected helper nodes by at least 10%, and retain useful
worker-0 foreign TT reuse. See the dedicated LMR experiment. Retirement needs
successful retirement activation in the strict diagnostic lane plus preserved
depth and retained contributions. Nonselected nodes are not all wasted work.

## NMP and the later root split

Baseline NMP diagnostics are emitted by the counter lane as per-worker
`v2_nmp` records and seven-column depth buckets. The parser rejects histogram
sum mismatches, duplicate buckets, invalid outcomes, and missing summary
records. Columns are attempts, fail-highs, verification starts, verification
successes, verification overturns, later exact-hash PV-node returns, and
tracking overflow. Bucket 31 includes depths 31 and above.
`prior_reductions` is separate and must remain zero until `T_V2_NMP_PRIOR=1` is
explicitly tested. PV returns describe a later search call in the same worker;
they do not assert that a null-move position entered the final returned PV.

Only after the earlier factors have been diagnosed should prior-NMP run as a
single factor, with `T_V2_NMP_PRIOR=0` versus `1`. Its bounded correction is an
experimental branch decision, so fixed-depth node identity is not its gate.
Verification overturns by depth must be reviewed before any strength test.

The cooperative root prototype is a separate, exactly-two-worker lane
(`LazyFlags=16399`), not a Lazy SMP result-selector experiment. Its aggregate
worker result has different accounting; the strict Lazy SMP worker parser and
selected-worker TT-origin checks must not be used to qualify it. It needs its
own candidate/main-confirmation diagnostics and correctness checks before
resource or strength claims.

## Independent gate

Select at most one candidate after complete diagnostics. Do not open its gate
while the legacy depth-consensus process is active. Freeze a new, unused,
Sekirei-generated opening set, exact binaries and option hashes, colour
reversal, four workers, `Hash=64`, material evaluation, equal wall-time, and
paired trinomial SPRT H0=0 Elo/H1=+10 Elo with alpha=beta=0.05 before any games.
H1 is required for adoption; H0 rejects the candidate, and a capped or
interrupted run is inconclusive. Diagnostics, the legacy gate, alternative
candidates, and restarted runs must not be pooled.

## Measured identity checks (2026-10-14)

Final tune executable SHA-256 is
`2bf06dd37e93237f54ef3a5dab4295920d6d53b5b453601e7195aa01716b1246`.
On all 20 frozen positions, one-worker eager/deferred comparisons at depths 6
and 8 matched bestmove, score, PV, nodes and depth in every ABBA block. Four
workers showed within-side nondeterminism on all 20 positions at each depth,
so no four-worker node-identity claim follows.

The separate pristine-HEAD58347 versus candidate default comparison also
matched all 20 one-worker depth-8 signatures. Its original build label was
corrected after discovering the baseline was pristine HEAD rather than the
intended initial dirty snapshot; the executable hashes and raw results are
unchanged. This is not the corrected initial-dirty regression lane.

See `identity-results.json` for exact artifact references. Concurrent builds
and tests were observed during these identity runs. Their timing is unqualified;
these results establish neither NPS improvement nor candidate advancement.

The corrected initial-dirty baseline was subsequently rebuilt from the verified
`b0acddc` archive. Its executable SHA-256 is
`c10c22c84ace1e19819ca531964890ebe546839c61b2573facc4da68c5e0eeda`.
Against the same final candidate, all 20 one-worker depth-8 ABBA blocks matched
the complete signature. `identity-results.json` also records the baseline
build-manifest hash. This corrected lane confirms sample-wise preservation
of the initial dirty default, still without a timing inference.

## Completed 300ms diagnostics

Every final timed lane explicitly used `MoveOverhead=0`. Results below are
per-factor diagnostics, never pooled across alternatives or reruns.
`diagnostic-results.json` contains raw side medians, medians of the ABBA block
mean differences, and separate A-first/B-first paired differences. Those
statistics answer different questions; a difference between raw medians is
not a paired depth gain.

On the four-worker counter lanes (20 positions, two repeats, 40 ABBA blocks):

| Factor | Raw median depth A / B | Median ABBA block-mean B-A | A-first / B-first depth delta | Outcome |
|---|---:|---:|---:|---|
| helper root LMR | 16 / 15.5 | -0.25 | 0 / 0 | Does not meet advancement criteria |
| helper root hint | 16 / 16 | 0 | 0 / 0 | Transfer activated; no depth gain established |
| helper retirement | 16 / 16.5 | 0 | 0 / -0.5 | No helper retired; no efficiency evidence |

LMR reduced the median paired helper-only nonselected-node count by 1.64%,
below the required 10%; helper completed-iteration delta was zero. Hinting
changed root order 34 times in 225 eligible reads (15.11%), across 19 of 80
candidate searches. Ten of those 34 applications produced a first-move beta
cut (29.41%). Its 52 aspiration retries accounted for 670,908 nodes, including
189,927 stale-research nodes. These are attribution counters, not causal
estimates. Invocation generation gap remained zero; completed-depth gap was
at most two plies.

The separate hint scaling run used 1/2/4/8 workers and one repeat. Raw selected
depth medians A/B were 17/17, 17/17, 16/16 and 15.5/16. Every thread count had
a median ABBA block-mean depth delta of zero; four workers had a B-first
median paired delta of -0.5. This does not show the requested stable 0.5-1 ply
improvement. The host is Apple M4 with ten physical/logical cores.

The final bounded sort run observed paired wall-NPS ratios of 1.00301 at one
worker and 1.00574 at four workers (engine-reported NPS ratios 1.00279 and
1.00522). Its start snapshot was idle, but the end snapshot found
`mobileassetd` and `deleted` above 90% CPU, so timing was marked unqualified.
An earlier separate 300ms run found a compiler active at its end; transient
OS/compiler activity also blocked several unopened attempts. A 250ms-effective
run using the historical 50ms overhead was retained as exploratory and
excluded from the final 300ms evidence. No repeated run is pooled to improve
the result. There is no verified speed gain from these measurements.

The exactly-two-worker cooperative prototype completed the four-position
fixed-depth smoke. Across eight candidate searches, helpers produced 831
probes and 22 fail-high candidates; worker 0 performed 19 confirmations and
three confirmed alpha raises. Generation changes may discard candidates, so
publication and confirmation counts need not match. This shows real
participation, not a speed or strength result.

`nmp-baseline-results.json` profiles only the 80 baseline searches from the
LMR lane: 1,356,360 NMP attempts, 1,023,503 fail-highs, 8,622 verifications,
8,463 retained fail-highs and 159 overturns. Verification retained 98.16% of
completed results. There were 23,551 later same-worker exact-hash PV calls,
zero tracking overflows and zero prior-history corrections. The file includes
the complete depth distribution. No prior-NMP correction was adopted.

`pv-validation.json` records a frozen Rust rule-tool replay of all 1,616 rows
and 12,739 PV moves from the final selected runs: zero illegal transitions or
bestmove/PV-prefix violations. This checks the same-rules PV contract; it does
not independently prove rule correctness or mate scores.

No candidate advances or changes the default. LMR missed its criteria,
retirement did not activate, and hint depth/timing did not qualify. The legacy
independent gate also remains unsuitable for authorizing a new gate. No new
SPRT or NNUE training was started.

## Final local validation

Core/USI tests: 440 passed, six intentionally ignored; the run used the
repository's 16MiB test-stack setting and serial test execution. The targeted
Python suites passed 67 tests. The complete default workspace, opening-book,
tuned search matrix, actual NMP-prior correction, ignored concurrency/perft
contracts, native WASM API and tiny trainer fixture also passed the existing
coverage script. Engine/library line coverage was 37,854/38,597 (98.075%).
The exclusion expression is byte-for-byte unchanged. This is local coverage,
not hosted Codecov confirmation. See `validation.json` for commands and hashes.

No candidate was adopted and no new SPRT was opened. The NNUE training lane,
weights and evaluator defaults are unchanged. The Lazy SMP default remains
15 and the ancestor-NMP correction remains zero. See [later one-factor tuning](tuning-matrix.md)
for existing parameters and the retained-TT requirement for generation aging.
