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
The sort baseline is `V2_EAGER_LOSING_SORT=1` in the same tune executable.

Every paired run copies both executable streams and the complete input into a
new output directory, hashes those copied streams, and records effective
positions, explicit options, material-only evaluator, budget, host, and start
time. A supplied source label is recorded as unverified: an executable hash
alone does not prove which source built it. Preserve build logs and dirty-source
snapshots alongside the runner's manifest.

## Order and measurements

`scripts/paired_search_diagnostics.py` runs **A, B, B, A on each position**
before advancing to the next position. Each search starts a fresh process and
uses `Hash=64`, no book, no speculative top-N, and material evaluation. Searches
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
  --a-option V2_EAGER_LOSING_SORT=1 --b-option V2_EAGER_LOSING_SORT=0 \
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
`prior_reductions` is separate and must remain zero until `V2_NMP_PRIOR=1` is
explicitly tested. PV returns describe a later search call in the same worker;
they do not assert that a null-move position entered the final returned PV.

Only after the earlier factors have been diagnosed should prior-NMP run as a
single factor, with `V2_NMP_PRIOR=0` versus `1`. Its bounded correction is an
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
