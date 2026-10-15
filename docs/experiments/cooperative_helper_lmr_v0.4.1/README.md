# Cooperative helper root LMR

COOP_HELPER_LMR=1 is an independent, default-off, two-worker cooperative-root
search candidate. Ordinary search, main-owned root reductions and main
confirmations are unchanged. No NNUE or training/hold-out data is used.

Only root depth >=3 and original ordered move index >=3 are eligible. Captures,
promotions, moves while in check and checking moves are excluded regardless of
ROOT_LMR. Reduction is root_lmr_reduction(..., true).min(1): at most one ply.
A reduced fail high is rechecked at full depth with the same task alpha and
null window before any candidate is published. Main still verifies candidates
at full depth against current alpha. Reduced false fail lows remain possible;
this is a selective-search experiment, not a behavior-preserving optimization.

The legacy helper_probes counter retains full-depth completed-probe semantics:
reduced fail lows do not increment it. Both stages' nodes are attributed once
to the completed or aborted helper task. Separate optional counters report
reduced attempts, reduced fail highs, full-depth recheck starts, completed
full-depth fail highs and completed reduced fail lows. They are allocated and
updated only with COOP_HELPER_LMR=1 and COOP_LATENCY=1. A separate activation
line identifies the enabled policy even with counters disabled. Existing USI
lines and public search entry points remain compatible.

## Prospective development protocol

Save and commit this protocol before searches. Freeze a clean standard native
release tune build, source/compiler/config/environment/log and engine hashes.
Use the existing twenty development positions (10 quiet, 10 tactical), SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
These positions cannot qualify independent playing strength.

Common controls: material evaluation, Hash64, no book, MoveOverhead0, SpecTopN0,
SearchMode=LazySMP, ASP_VOLATILITY=0, V2_SORT=0, COOP_DRAIN=0,
COOP_PERSISTENT=0 and COOP_PRIORITY=0. Only COOP_HELPER_LMR differs in A/B.

1. Default-off identity: frozen source30a8ab0 engine SHA-256
   `5f98a4626be7d6d7e95bd90acd99aa2fc7b390d6a56c68322723f0059117f536`
   versus new engine, 1T, LazyFlags15, depth8, repeat1 per-position ABBA,
   80 searches. Require exact bestmove/score/PV/nodes/depth identity.
2. Mechanism: same new engine both arms, 2T, LazyFlags16399, depth10,
   repeat1 ABBA, 80 searches, COOP_LATENCY=1. Require correct activation,
   legal PVs, complete thread/node accounting and positive reduced probes,
   completed reduced fail lows and full-depth rechecks. Counters are mechanism
   observations and cannot measure speed.
3. Timing: same engine, 2T, LazyFlags16399, 300ms, repeat2 ABBA, 160 searches,
   COOP_LATENCY=0. Require pre/post idle-host guards and correct activation.
   Allow at most two attempts only when the first blocks before any search,
   separated by >=60 seconds. Never retry a completed/interrupted run based
   on its outcome or host-after state; never stop other jobs or bypass guards.

A candidate advances this development screen only with qualified timing,
overall median paired completed-depth delta >=0.5 ply, positive medians in
both ABBA directions and nonnegative quiet/tactical subgroup medians. NPS
cannot rescue depth failure. Replay all completed PVs with the frozen legal
validator; this is a same-rules check, not an independent rule oracle.

This compares cooperative variants, not YaneuraOu. Defaults remain OFF until
independent H1. The existing queued aspiration candidate and its owner-attested
identity exclusions are unchanged; this diagnostic cannot replace that gate.

## Reproduction

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline <new-engine> --candidate <same-engine> \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output <new-dir> --threads 2 --count 20 --repeats 1 --depth 10 \
  --option LazyFlags=16399 --option T_COOP_LATENCY=1 \
  --option T_COOP_DRAIN=0 --option T_COOP_PERSISTENT=0 \
  --option T_COOP_PRIORITY=0 --option T_ASP_VOLATILITY=0 --option T_V2_SORT=0 \
  --a-option T_COOP_HELPER_LMR=0 --b-option T_COOP_HELPER_LMR=1 \
  --source-ref <built-commit>
```

For timing use COOP_LATENCY=0, movetime300 and repeats2. Do not add ordinary
--diagnostics: cooperative aggregate output differs from independent-worker
records. The committed source preserves this protocol before searches.


## Completed observations

Clean source `a1822b6d641749081d2c2357552f7983c069c204` produced the standard
native tune engine SHA-256
`7183870a09c6bf7792eb7c5a69e1d11263fad599e15cbe9db19a4000c90c1bfa`.
[results.json](results.json) binds source, build receipts, scripts, tests, raw
rows/manifests/summaries, PV validation and resource-only timing attempts.

All 20 default-off ABBA blocks (80 searches) matched bestmove, score, PV,
nodes and depth exactly. The 2T depth10 mechanism lane completed 80 searches
with correct activation in all rows. Candidate helpers started 5,627 reduced
probes, completed 5,345 reduced fail lows and started 276 full-depth rechecks;
245 rechecks completed with fail high. The remaining rechecks include fail lows
or interruption and are not all claimed as overturned advice. Legacy probe
counts retain their full-depth meaning, so reduced fail lows must not be
compared as missing full-depth helper work. These counter-lane observations
establish activity, not saved nodes or speed.

All 1,258 PV moves across 160 searches replayed legally. Both 300ms timing
attempts blocked before any search: first on OS service load, second on an
unrelated compiler. The resource-only retry gap was 85.177236 seconds. No third
attempt was allowed, no processes were stopped and no load guard was bypassed.
There are zero timed rows, so completed-depth and NPS improvements are unmeasured.
The development screen cannot advance without qualified timing; this does not
establish that the candidate is weaker. No SPRT was started and the existing
independent aspiration queue is unchanged.

Core 365 tests passed with 4 ignored, tune USI 26 and related Python 81 passed.
Formatting, workspace default Clippy, tune USI/core Clippy and documentation
references passed. The helper-TT fixture uses child depths >5 to avoid V2's
intentional shallow expected-cut TT filter, and notification tests provide the
required wake channel. Earlier fixture failures are retained separately.
This is not the ten-million-random-validation or 64-core contention contract.
COOP_HELPER_LMR remains 0; no strength gain or YaneuraOu-superiority claim follows.
