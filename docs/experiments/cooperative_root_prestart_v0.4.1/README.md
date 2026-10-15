# Start cooperative helpers before the first root child returns

Prospective default-off experiment on base
`eea038febf94843deff606aa2449e37e62f352f7`. Commit the implementation and this
protocol before native builds or benchmark searches. Freeze all measurement
code, including build/analysis/collection drivers, before the first search.
Preserve this original text as `target/cooperative-root-prestart-prospective-protocol.md`
and bind it, all drivers, inputs, receipts and executables in the presearch
receipt. No NNUE, training labels or hold-out contents are read or changed.

The single candidate `COOP_PRESTART=1` sends generation-scoped root jobs to
persistent helpers before worker0 searches the first root child. It applies
only to existing COOP_MULTI persistent groups at Threads4/8, depth>=2 and a
finite, non-mate aspiration bounds. Full-window passes, Threads1/2,
scoped executors and independent Lazy SMP retain their existing behavior.
Helpers probe later root moves against the aspiration lower bound, then
worker0 verifies every fail-high candidate against its current alpha. The
first child retains its full current aspiration window; a first-child beta
cut or cancellation stops the generation. Root claims stay disjoint and
all threads join before go returns. No helper result directly sets the PV.

`COOP_PRESTART` and its independent observer `COOP_PRESTART_DIAG` default0.
The observer records adjacent marker observations before the first child's
return: job starts, completed probes, fail-highs, private first-child nodes,
first-child time and helper nodes on first-child beta cuts. These markers do
not measure precise CPU overlap. Dispatch-order tests and the actual probe
observations provide separate evidence. The observer is disabled for every
speed/depth qualification run. No new pruning or default adoption is included.

## Frozen builds and inputs

Use clean native release normal/tune builds from the same committed source,
Rust1.97.0, Cargo configuration and environment, without PGO. Require at least
768MiB free, CARGO_INCREMENTAL=0 and CARGO_BUILD_JOBS=1. Preserve the normal
executable before building tune. Receipts bind source, features, compiler,
config, environment, protocol, log and executable hashes. Never overwrite an
existing build or measurement directory.

```sh
python3 target/build-cooperative-root-prestart.py normal --source-ref <source>
python3 target/build-cooperative-root-prestart.py tune --source-ref <same-source>
```

Outputs: `target/cooperative-root-prestart-build/{normal,tune}/engine`,
`build.log` and `receipt.json`. Generic paired-driver source_ref_verified=false
is separate from these clean source/build receipts.

Previous normal control: `target/cooperative-persistent-group-build/normal/engine`,
source `2db51b4628ae284d800b8304892751ab426634dc`, SHA-256
`44707e344889eedff100cf163c539b1be0b1bf4b231a7232123ad6d63fd28444`.
Verify matching compiler/config receipts and no production difference from
that source to this experiment's base.

Retain all20 existing development positions, 10quiet and10tactical:
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
This development corpus is not an independent strength gate. Do not enable
ordinary --diagnostics: cooperative aggregates differ from independent-worker
records. All slots use fresh engine processes, material evaluation, Hash64,
no book, MoveOverhead0, SearchMode=LazySMP and SpecTopN0; the paired driver pins
the latter controls internally. Never time normal versus tune.

## Prospective lanes and screens

1. Default identity: old frozen normal versus new normal, LazyFlags15,
   Threads1/4/8, depth8, one ABBA repeat per position: 240rows. All20 one-thread
   positions must match bestmove, score/unit/bound, PV, nodes and completed
   depth exactly. Report parallel within-arm and between-arm variation
   separately. No root group/prestart may activate. Replay every PV before
   proceeding.
2. Mechanism: same new tune executable in both arms, Threads1/2/4/8, depth8,
   one ABBA repeat: 320rows. Common options: LazyFlags16399, COOP_MULTI1,
   COOP_PERSISTENT1, COOP_DRAIN0, COOP_PRIORITY0, COOP_LATENCY0,
   COOP_HELPER_LMR0, COOP_PRESTART_DIAG0, SEARCH_V2=1, MoveOverhead0.
   Only COOP_PRESTART differs, A0/B1. All20 1T controls must be exact.
   Threads2 retains the existing persistent prototype and is a control.
   Both4T/8T arms must activate groups workers=N/helpers=N-1. Both must start
   all3/7 physical helpers when jobs run, reuse them across generations, have
   zero spawn failures and join all physical threads. Require job census
   started+spawn_failures=generations*(N-1), completed+aborted helper nodes=
   helper nodes, main+helper nodes<=invocation nodes, confirmation private
   nodes<=main nodes, confirmations<=fail-highs<=probes and raises<=confirmations.
   A must have no prestart record; B4/8 must have one, with aggregate passes>0
   independently at each count. Per-go passes0 is allowed for full-window
   fallback. No observer record is allowed. Threads1/2 must have no prestart
   records. Preserve all rows and replay every PV.
3. Observer-only diagnostic lane: after mechanism validation, same tune A/B,
   common mechanism options except COOP_PRESTART_DIAG1, Threads4/8, depth8,
   one ABBA repeat: 160rows. A has no prestart observer; B records it.
   Require positive aggregate observed helper jobs and completed probes
   before first-child return independently at4T/8T; report fail-highs even
   when zero. Verify first-main nodes are a private child subset of all main
   nodes and first-beta helper nodes do not exceed prestart helper nodes.
   Basic prestart nodes cannot exceed all helper nodes. Replay every PV.
   Observer rows must flag counter overhead and timing_qualified=false.
   Do not use diagnostic-lane wall time or NPS for speed/depth qualification.
4. Timing: after the three completed validated lanes, same tune A/B and
   mechanism options, explicitly COOP_PRESTART_DIAG0, Threads1/2/4/8, 300ms,
   two ABBA repeats: 640rows. Pre/post host checks must both be available and
   idle at the25-percent busy-process threshold. At most two attempts;
   attempt2 is allowed only if attempt1 blocked before any search and
   at least60seconds elapsed. Never retry a completed or interrupted run,
   including post-host invalidation. Never stop other jobs, use busy override,
   change corpus/options or run a third attempt. A permitted completed retry
   is qualified by its own pre/post guards; an earlier presearch block alone
   does not invalidate it. Preserve blocked and negative observations.

Qualify4T and8T independently. Each advances only with qualified timing,
median paired completed-depth delta>=0.5ply, nonnegative quiet/tactical
medians and a strictly positive95-percent position-bootstrap lower bound for
mean completed-depth delta. Average each position's ABBA paired deltas over
two repeats, resample20positions with replacement5000times, seed20261015,
using sorted percentiles at floor(p*(draws-1)). Failure at4T cannot reject an8T
pass, and vice versa. Threads1/2 remain controls. Preserve both ABBA directions,
all depth distributions, prestart/job/node census, physical-thread reuse and
raw wall NPS in ordinary lanes. NPS cannot rescue a depth failure.

Timing/depth diagnostics do not establish Elo or YaneuraOu superiority.
Defaults remainOFF pending independentH1. Existing independent-strength gates
retain their owner-attested diagnostic/previous-strength-gates/nnue-holdouts
identity manifest requirements; this experiment does not replace them.

## Reproduction

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline target/cooperative-persistent-group-build/normal/engine \
  --candidate target/cooperative-root-prestart-build/normal/engine \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output target/cooperative-root-prestart-default-identity \
  --threads 1,4,8 --count 20 --repeats 1 --depth 8 \
  --option LazyFlags=15 --option MoveOverhead=0 --source-ref <source>

python3 scripts/paired_search_diagnostics.py \
  --baseline target/cooperative-root-prestart-build/tune/engine \
  --candidate target/cooperative-root-prestart-build/tune/engine \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output target/cooperative-root-prestart-mechanism \
  --threads 1,2,4,8 --count 20 --repeats 1 --depth 8 \
  --option LazyFlags=16399 --option T_COOP_MULTI=1 \
  --option T_COOP_PERSISTENT=1 --option T_COOP_DRAIN=0 \
  --option T_COOP_PRIORITY=0 --option T_COOP_LATENCY=0 \
  --option T_COOP_HELPER_LMR=0 --option T_COOP_PRESTART_DIAG=0 \
  --option T_SEARCH_V2=1 --option MoveOverhead=0 \
  --a-option T_COOP_PRESTART=0 --b-option T_COOP_PRESTART=1 \
  --source-ref <source>
```

For the diagnostic lane retain mechanism inputs/options, use output
`target/cooperative-root-prestart-diagnostic`, Threads4/8 and
T_COOP_PRESTART_DIAG1. For timing retain the counter-free mechanism options,
use new `target/cooperative-root-prestart-timed-attempt1`, remove --depth8,
use --movetime300 and --repeats2.

Replay each completed lane separately using the frozen diagnostic legal tool,
SHA-256 `dfbe08d01ced743c0bbd409fe0ca816599d53daa4ef6dcbaa447e381c8d9dc00`:

```sh
python3 scripts/validate_paired_pv.py --legal-moves <frozen-legal-tool> \
  --output <run>/pv-validation.json <run>
python3 target/analyze-cooperative-root-prestart.py <run> \
  --mode <identity|mechanism|diagnostic|timing> --source-ref <source>
python3 target/collect-cooperative-root-prestart.py --source-ref <source>
```

The analyzer validates complete240/320/160/640-row ABBA coverage, frozen builds,
options, inputs, actual mechanism counters and legal replay receipts. Each
successive lane requires validated earlier lanes. Reports normalize JSON keys;
collection recomputes the saved analyses and verifies all bound hashes.
Collection preserves finalized raw files and excludes its own growing output,
future mutable CI/watch/PR/integration captures. Existing evidence is verified
without overwriting. The same-rules legal replay is not an independent movegen
oracle or mate proof. alpha_gap_max totals sum per-go maxima and cannot be
called a global maximum.

## Observed result (2026-10-15)

Frozen native source: `d7d4a5bc857ebf9d17432780841e46bf7e614207`.
The 240-row default identity, 320-row mechanism and 160-row observer lanes
completed. All20 one-thread default and mechanism positions retained exact
bestmove/score/PV/nodes/depth identity. Persistent4T/8T group census, generation
completion and physical joins passed. All720 PV rows (4938 moves) replayed
legally with the frozen same-rules tool; this is not an independent oracle.

| Observer aggregate | 4T | 8T |
|---|---:|---:|
| Eligible prestart passes | 466 | 473 |
| Helper job starts before first-child return marker | 1367 | 3235 |
| Completed helper probes before marker | 10396 | 12468 |
| Helper fail-highs before marker | 570 | 752 |
| First-child beta cuts | 74 | 83 |
| Helper nodes in first-child beta-cut passes | 17785 | 18474 |

These observations establish activation, not useful-depth or speed gains.
Both permitted300ms timing attempts blocked before any search because other
processes exceeded the25-percent busy threshold (retry gap60.089894seconds).
Timing has0completed searches, is unqualified and has no4T/8T advancement
verdict. No further attempt belongs to this frozen protocol. Other jobs were
not stopped. A future timing run needs a new prospective measurement record.

Local validation:406core tests passed (4ignored), two isolated tune activation
tests passed,79measurement-tool tests passed, USI wire-format test passed,
and default workspace/tune-core Clippy passed. Detailed receipts, both blocked
attempts and90hashed artifacts are bound in [results.json](results.json).
`COOP_PRESTART` and `COOP_PRESTART_DIAG` remainOFF; independentH1 and
YaneuraOu superiority are unproven.
