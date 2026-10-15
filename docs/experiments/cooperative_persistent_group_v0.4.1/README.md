# Persistent cooperative root groups

Prospective default-off experiment on base
`ac5d45dd1b9c74306f949f6a1fb0a3468be817ee`. Commit the implementation and this
protocol before native builds or benchmark searches. Preserve this prospective
text and bind all drivers, input files, receipts and executables before the
first benchmark search. No NNUE, training labels or hold-out contents are read
or changed.

The candidate extends COOP_MULTI root groups at Threads4/8 to the existing
COOP_PERSISTENT executor option. Worker0 retains the first full-window child
and confirms every helper fail-high candidate with the current alpha. Each
helper has private mutable search state; atomic root indices remain disjoint.
Persistent helpers live only for one go, receive generation-scoped jobs, and
all join before that go returns, including cancellation and panic paths.
COOP_MULTI remains default0 and COOP_PERSISTENT remains default0. No new
pruning, evaluator, task-selection policy or default adoption is included.

## Frozen builds and inputs

Use normal and tune native release builds from the same clean committed source,
compiler, Cargo configuration and environment, without PGO. Require at least
768 MiB free, CARGO_INCREMENTAL=0 and CARGO_BUILD_JOBS=1. Preserve the normal
executable before building tune. Build receipts bind source, features,
compiler/config, environment, protocol, log and executable hashes:

```sh
python3 target/build-cooperative-persistent-group.py normal --source-ref <source>
python3 target/build-cooperative-persistent-group.py tune --source-ref <same-source>
```

Outputs: `target/cooperative-persistent-group-build/{normal,tune}/engine`,
`build.log` and `receipt.json`. Never overwrite an existing output. The generic
paired driver's source_ref_verified=false is not a substitute for clean
source/build receipts.

Previous normal control: `target/cooperative-root-multi-build/normal/engine`,
source `4707c1a51dd5e10a70f925fd29ce567292e64f80`, SHA-256
`d76018507dc884f48520f1061d510258f008c73d82caa000859617e25614aedd`.
Verify matching compiler/config receipts and that the difference from that
source to the new base contains no production changes.

Retain all 20 existing development positions, 10 quiet and 10 tactical:
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
This is a development corpus, not an independent strength gate. Do not add
ordinary --diagnostics: cooperative aggregate counters differ from the
independent-worker diagnostic records. All slots use fresh engine processes,
material evaluation, Hash64, no book, MoveOverhead0, SearchMode=LazySMP and
SpecTopN0; the paired driver pins the latter controls internally.

## Prospective lanes

1. Default identity: old frozen normal versus new normal, LazyFlags15,
   Threads1/4/8, depth8, one ABBA repeat per position: 240 rows. Require all20
   one-thread positions to match bestmove, score/bound, PV, nodes and completed
   depth exactly. Report parallel within-side and between-arm variation
   separately. No root group may activate. Replay every PV before proceeding.
2. Mechanism: same new tune executable in both arms, Threads1/2/4/8, depth8,
   one ABBA repeat: 320 rows. Common options: LazyFlags16399, COOP_MULTI1,
   COOP_DRAIN0, COOP_PRIORITY0, COOP_LATENCY0, COOP_HELPER_LMR0, SEARCH_V2=1,
   MoveOverhead0. Only COOP_PERSISTENT differs, A0/B1. Require all20 1T controls
   to remain exact. Threads2 uses its existing scoped/persistent prototype;
   its process reuse may change and is a control observation, not an identity
   requirement. Both 4T/8T arms must actually activate groups workers=N,
   helpers=N-1. A starts one OS thread per helper job; B starts at most3/7
   OS threads per go and reuses them across generations. Require all configured
   B helpers to start when any generation runs, and observe more jobs than
   physical threads across the lane. No fallback qualifies as group activation.
   Require started+spawn_failures=generations*(N-1), zero spawn failures for
   full-worker qualification, all physical threads joined, classified
   completed+aborted helper nodes=helper nodes, main+helper nodes<=invocation
   nodes, and main confirmation/probe bounds. Preserve all rows and replay
   every PV.
3. Timing: only after identity and mechanism validation, same tune A/B and
   options, Threads1/2/4/8, 300ms, two ABBA repeats: 640 rows. Pre/post host
   checks must both be available and idle at the25-percent busy-process
   threshold. At most two attempts; attempt2 is allowed only after attempt1
   blocked before any search and at least60seconds elapsed. Never retry a
   completed or interrupted run, including post-host invalidation. Do not stop
   other jobs, use busy override, change corpus/options or run a third attempt.

Qualify 4T and 8T independently. Each advances only with qualified timing,
median paired completed-depth delta>=0.5ply, nonnegative quiet and tactical
medians, and a strictly positive95-percent position-bootstrap lower bound
for mean completed-depth delta. Average each position's ABBA paired depth
changes over the two repeats, resample20positions with replacement5000times,
seed20261015, sorted percentiles at floor(p*(draws-1)). Failure at4T cannot
reject an8T pass, and vice versa. Threads1/2 are controls. Preserve both ABBA
directions, full depth distributions, jobs/physical-thread ratio and raw wall
NPS. NPS has no acceptance threshold and cannot rescue a depth failure.

Timing/depth diagnostics do not establish Elo or YaneuraOu superiority. Defaults
remain OFF pending independent H1. The separate queued aspiration/independent
YaneuraOu gates retain their existing owner-attested three-scope identity
manifest requirements; this experiment does not substitute for those gates.

## Reproduction

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline target/cooperative-root-multi-build/normal/engine \
  --candidate target/cooperative-persistent-group-build/normal/engine \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output target/cooperative-persistent-group-default-identity \
  --threads 1,4,8 --count 20 --repeats 1 --depth 8 \
  --option LazyFlags=15 --option MoveOverhead=0 --source-ref <built-source>

python3 scripts/paired_search_diagnostics.py \
  --baseline target/cooperative-persistent-group-build/tune/engine \
  --candidate target/cooperative-persistent-group-build/tune/engine \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output target/cooperative-persistent-group-mechanism \
  --threads 1,2,4,8 --count 20 --repeats 1 --depth 8 \
  --option LazyFlags=16399 --option T_COOP_MULTI=1 \
  --option T_COOP_DRAIN=0 --option T_COOP_PRIORITY=0 \
  --option T_COOP_LATENCY=0 --option T_COOP_HELPER_LMR=0 \
  --option T_SEARCH_V2=1 --option MoveOverhead=0 \
  --a-option T_COOP_PERSISTENT=0 --b-option T_COOP_PERSISTENT=1 \
  --source-ref <built-source>
```

For timing retain mechanism inputs/options, use a new
`target/cooperative-persistent-group-timed-attempt1` output, remove --depth8,
use --movetime300 and --repeats2. Never time normal versus tune.

Replay each completed lane separately using the already frozen diagnostic
legal tool, SHA-256
`dfbe08d01ced743c0bbd409fe0ca816599d53daa4ef6dcbaa447e381c8d9dc00`:

```sh
python3 scripts/validate_paired_pv.py --legal-moves <frozen-legal-tool> \
  --output <run>/pv-validation.json <run>
python3 target/analyze-cooperative-persistent-group.py target/cooperative-persistent-group-default-identity --mode identity --source-ref <built-source>
python3 target/analyze-cooperative-persistent-group.py target/cooperative-persistent-group-mechanism --mode mechanism --source-ref <built-source>
python3 target/analyze-cooperative-persistent-group.py <timed-run> --mode timing --source-ref <built-source>
```

The analyzer validates complete240/320/640-row ABBA coverage, build/engine/input
hashes, assigned options, actual mechanism counters and legal replay receipts.
Mechanism requires the completed identity report; timing additionally requires
the completed mechanism report. Path overrides require those same frozen lanes.
Reports normalize JSON keys and record the two independent thread screens.
The legal replay uses the same rules as Sekirei; it is not an independent
move-generation oracle or mate proof. Counter totals for alpha_gap_max sum
per-go maxima and must not be described as a global maximum.
