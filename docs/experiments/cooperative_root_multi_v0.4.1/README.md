# Scoped cooperative root groups

This is a prospective, default-off development experiment on base
`94c5ffaf71d4f691259b8285dcda029d53263c32`. Commit the implementation and this
protocol before any native benchmark builds or searches. No NNUE, training labels, hold-out
contents or owner identity exclusions are read or changed.

`T_COOP_MULTI=1` enables the existing root task-sharing design at Threads4/8
only with LazyFlags bit16384, shared TT and `T_COOP_PERSISTENT=0`. Worker0 retains
the first full-window child and all accepted scores. The other 3/7 scoped
helpers claim disjoint root indices and return full-depth null-window advice
which worker0 must confirm. Other worker counts, isolated TT and persistent
executor combinations retain their ordinary fallback. Threads2 retains the
existing prototype. Default flags and COOP_MULTI=0 remain unchanged.

This candidate creates 3/7 scoped threads per aspiration pass. Unused
persistent Lazy SMP Searchers and additional private helper histories impose
memory and initialization costs. The change may fail to improve useful depth;
correct sharing and joined threads alone do not establish a speed or strength
benefit. No new pruning, evaluator or default adoption is included.

## Frozen builds

Build normal native release and tune native release separately from the same
clean committed source, compiler, Cargo configuration and environment, without
PGO. Preserve the normal executable before the tune build replaces the release
output. Both builds require at least 768 MiB free, `CARGO_INCREMENTAL=0` and
`CARGO_BUILD_JOBS=1`. The task-local build driver binds source, prospective
protocol, features, compiler/config, environment, log and executable hashes:

```sh
python3 target/build-cooperative-root-multi.py normal --source-ref <committed-source>
python3 target/build-cooperative-root-multi.py tune --source-ref <same-committed-source>
```

Outputs are `target/cooperative-root-multi-build/{normal,tune}/engine`,
`build.log` and `receipt.json`. Existing outputs are never overwritten.
A clean source/build receipt establishes the build identity; the generic
paired driver alone reports source_ref_verified=false and is not a build proof.

The frozen previous normal control is
`target/correction-key-unroll-build/A/engine`, source
`72706632774c15e6b3690d1847dd0fc465a643ca`, SHA-256
`59d99aabf1aaade97e742f994d3d919cd81a464aed40313da28a48ec6c448be4`.
Changes from 7270663 to base 94c5ffa are experiment documentation, with no
production code difference. Verify that difference and matching native build
receipts when analyzing default identity.

## Common measurement contract

Use all 20 existing Sekirei development positions, 10 quiet and 10 tactical:
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Keep every position and record both ABBA directions. This development corpus
cannot qualify independent playing strength and must not enter a future
independent gate.

All lanes use fresh engine processes per slot, material evaluation, Hash64,
no book, MoveOverhead0, SearchMode=LazySMP and SpecTopN0. The paired driver pins
the latter mode and speculative controls internally. Do not add ordinary
`--diagnostics`; cooperative aggregate counters are intrinsic and differ from
independent-worker diagnostic records.

For mechanism/timing, use the same new tune executable in both arms. Common
options are LazyFlags16399, COOP_PERSISTENT0, COOP_DRAIN0, COOP_PRIORITY0,
COOP_LATENCY0, COOP_HELPER_LMR0 and SEARCH_V2=1. Only COOP_MULTI differs, A0/B1.
No latency observer or per-node counter overhead is included. Intrinsic root
job/node totals still validate task ownership, completion and classification.

## Prospective lanes and gates

1. Default preservation comes first: old frozen normal versus new normal,
   LazyFlags15, Threads1/4/8, depth 8, repeat 1 per-position ABBA: 240 rows.
   Require exact bestmove/score/PV/nodes/depth identity for all 20 one-thread
   positions. Report parallel within-side and between-arm variation separately;
   parallel search is not a deterministic node-identity oracle. No group may
   activate. Replay every PV legally before proceeding.
2. Mechanism after default identity: same new tune binary A/B, Threads1/2/4/8,
   depth 8, repeat 1 ABBA: 320 rows. Require all 20 one-thread controls to remain
   exact, the legacy 2T prototype in both arms, no split for A4/8, and group
   workers=N/helpers=N-1 for B4/8. Check job census
   `started + spawn_failures = generations * (N-1)`, all physical threads joined,
   scoped job/thread equality, and completed+aborted helper nodes=helper nodes.
   Classified main+helper nodes must not exceed total invocation nodes. Preserve
   all rows and validate every PV.
3. Fixed-time screen after mechanism: same tune A/B and common options,
   Threads1/2/4/8, 300ms, repeat 2 ABBA: 640 rows. Both pre/post host checks must
   be available and idle using the 25-percent busy-process threshold. At most
   two attempts are allowed, with the second only after the first blocked
   before any search and at least 60 seconds elapsed. Never retry a completed
   or interrupted run, including post-host invalidation. Do not stop other
   jobs, use busy override, or change positions/options after observing results.

An 8T development screen advances only if timing is qualified, overall median
paired completed-depth delta is at least 0.5 ply, quiet and tactical medians
are nonnegative, and the 95-percent position-bootstrap lower bound for the mean
completed-depth delta is strictly positive. Average each position's paired
ABBA depth deltas over its two repeats before bootstrap; resample 20 positions
with replacement, 5000 draws, seed 20261015, sorted percentiles at
`floor(p * (draws-1))`. The 4T overall/quiet/tactical medians must all be
nonnegative. Threads1/2 are control observations. Preserve and report both
ABBA directions, full depth distributions and raw paired NPS; NPS has no
acceptance threshold and cannot rescue a depth failure.

Qualified timing is necessary but not playing-strength evidence. Defaults
remain OFF pending independent H1. Independent YaneuraOu testing and the queued
aspiration candidate still require their existing owner-attested three-scope
identity manifests. This experiment neither substitutes for nor changes that
queue.

## Reproduction

Default identity:

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline target/correction-key-unroll-build/A/engine \
  --candidate target/cooperative-root-multi-build/normal/engine \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output target/cooperative-root-multi-default-identity \
  --threads 1,4,8 --count 20 --repeats 1 --depth 8 \
  --option LazyFlags=15 --option MoveOverhead=0 --source-ref <built-source>
```

Mechanism:

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline target/cooperative-root-multi-build/tune/engine \
  --candidate target/cooperative-root-multi-build/tune/engine \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output target/cooperative-root-multi-mechanism \
  --threads 1,2,4,8 --count 20 --repeats 1 --depth 8 \
  --option LazyFlags=16399 --option T_COOP_PERSISTENT=0 \
  --option T_COOP_DRAIN=0 --option T_COOP_PRIORITY=0 \
  --option T_COOP_LATENCY=0 --option T_COOP_HELPER_LMR=0 \
  --option T_SEARCH_V2=1 --option MoveOverhead=0 \
  --a-option T_COOP_MULTI=0 --b-option T_COOP_MULTI=1 --source-ref <built-source>
```

For timing retain that command's inputs/options and change output to a new
`target/cooperative-root-multi-timed-attempt1` directory, remove `--depth 8`,
use `--movetime 300` and `--repeats 2`. The retry restrictions above apply.
Never time the normal and tune binaries against each other.

Replay each completed lane separately before analysis, using the already
frozen diagnostic legal tool (SHA-256
`dfbe08d01ced743c0bbd409fe0ca816599d53daa4ef6dcbaa447e381c8d9dc00`):

```sh
python3 scripts/validate_paired_pv.py --legal-moves <frozen-legal-tool> \
  --output <run>/pv-validation.json <run>
python3 target/analyze-cooperative-root-multi.py target/cooperative-root-multi-default-identity --mode identity
python3 target/analyze-cooperative-root-multi.py target/cooperative-root-multi-mechanism --mode mechanism
python3 target/analyze-cooperative-root-multi.py <timed-run> --mode timing
```

Run only the corresponding analysis mode for each lane. The analyzer validates
240/320/640 rows, frozen build/engine/corpus bindings, options, ABBA coverage,
actual group/job/node census and legal replay. Mechanism analysis requires the
completed default-identity report; timing analysis also requires the completed
mechanism report. Override their directory paths only with `--identity-run`
and `--mechanism-run` when the same frozen lanes were stored elsewhere.
Unqualified or malformed inputs cannot advance. These are development screens,
not an independent oracle, Elo result or proof of YaneuraOu superiority.

## Completed observation (2026-10-15)

Frozen source: `4707c1a51dd5e10a70f925fd29ce567292e64f80`.
Normal native SHA-256:
`d76018507dc884f48520f1061d510258f008c73d82caa000859617e25614aedd`;
tune native SHA-256:
`851e06a1cf4536082a3e05695e9445b916fd509509c7ddf6d66c0de043da5e2f`.
Both clean builds use Rust/Cargo 1.97.0, the same native configuration and no
PGO. The pre-search binding preserves the committed prospective protocol and
all analysis inputs. Final observations are in [results.json](results.json),
with SHA-256 and sizes for 74 local evidence artifacts.

Core tests: 389 passed, 4 ignored; the isolated tune activation/fallback matrix
passed separately. The expanded shallow sequential comparison includes a mate
position. All 41 legacy root-split tests, the 10-test multi-name selection
(including eight new tests), tune Clippy and the workspace commit hook passed.
Measurement tooling: 69 tests passed (including the 46 parser tests).

Default identity retained all 240 rows: all 20 one-thread positions matched
bestmove, score/bound, PV, nodes and depth exactly. Parallel arms varied within
each arm on all 20 positions at both 4T and 8T; no deterministic parallel node
identity is asserted. The 320-row mechanism lane activated only the intended
4T/8T groups, retained the legacy 2T path and passed all job/node/join checks.

The first timed attempt completed all 640 searches with both 25-percent CPU
checks idle; no retry was run or remains permitted. Material-only, 300ms,
20 positions, two ABBA repeats per position:

| Threads | Median paired depth delta | Quiet | Tactical | Mean position delta | Position-bootstrap 95% interval | Raw wall-NPS ratio |
| --- | ---: | ---: | ---: | ---: | --- | ---: |
| 1 | 0 | 0 | 0 | 0 | [0, 0] | 1.0004 |
| 2 | 0 | 0 | 0 | 0 | [-0.2500, 0.2375] | 1.0106 |
| 4 | -0.5 | -0.25 | -0.5 | -0.4250 | [-0.7125, -0.1500] | 0.2663 |
| 8 | +0.5 | +0.5 | +0.75 | +0.5375 | [0.2000, 0.8750] | 0.1886 |

8T meets its depth criteria in this development corpus; its A-first/B-first
medians were +1.0/0.0, so order sensitivity remains visible. The combined screen
**does not advance**, because 4T regressed. NPS is reported as raw node
throughput and is not an acceptance criterion. These are completed-depth
observations, not equivalent-tree work or playing-strength measurements.

Timed B4T started and joined all 7,458 helper jobs across 2,486 generations;
B8T started and joined all 16,548 across 2,364 generations. Neither had a spawn
failure. Worker0 confirmed 2,575/3,031 helper candidates at 4T/8T; 224/352
confirmations raised alpha. All 1,200 rows across the three lanes replayed
10,944 legal PV moves with zero violations using the frozen same-rules tool.
This replay is not an independent move-generation oracle or a mate proof.

COOP_MULTI remains default OFF. No independent H1, NNUE change, Elo claim or
YaneuraOu superiority claim is made. The measured 4T regression must be
resolved in a separately frozen candidate before this combined screen can
advance; this completed run must not be repeated to seek a favorable sample.
