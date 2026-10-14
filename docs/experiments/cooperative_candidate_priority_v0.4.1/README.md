# Cooperative root candidate priority

This is a disabled-by-default root ordering experiment, not a new pruning rule
or a playing-strength result. No NNUE weights, training or hold-out data are
used. COOP_PRIORITY=1 applies only to the two-worker cooperative prototype;
COOP_PRIORITY=0 keeps its existing ascending-index confirmation traversal.
Ordinary Lazy SMP defaults remain unchanged.

## Policy and correctness

Before each helper-candidate confirmation, the enabled policy scans the ready
slots and chooses the highest observed eligible reported LOWER score; ties
keep the lower root index. It consumes only that candidate using the existing
Acquire operation and rescans after each confirmation. Invalid generation or
depth slots must not block valid advice or inflate reorder counts. An empty
scan returns immediately; the policy never waits for future publications.

This is one coupled highest-ready/rescan policy. Its effect cannot be ascribed
solely to sorting: unlike the old linear pass it also checks arrivals again
between confirmations. Slot reads are observations across a scan, not an
instantaneous globally maximal snapshot. Different alpha snapshots and LOWER
bounds make the ranking heuristic imperfect; every eligible candidate still
requires worker-0 full-depth verification at the latest alpha. No helper score
is adopted directly, no active main child is interrupted, and root assignment,
LMR, budgets, cancellation, mandatory helper join and final drain remain intact.

The original nineteen-field cooperative_root_latency line remains unchanged.
With COOP_LATENCY=1, an additional outcomes snapshot records confirmation starts
and actual main-verified beta cuts by arrival phase, regardless of whether the
helper reported a bound at beta. It also records priority choices/reorders and
the alpha raises, cuts and private confirmation nodes of reordered choices.
A reorder means choosing an index other than the lowest observed eligible
ready index in the same scan. Interrupted confirmation nodes remain counted;
aborted results never count as confirmed alpha raises or cuts.

Outcome instrumentation is absent when COOP_LATENCY=0. The independent
cooperative_root_policy priority 1 line records requested activation even in
counter-free timing. Existing legacy stats layouts and pair/triple search
APIs remain compatible; additive snapshots are non-exhaustive. Outcome node
counts are actual selected confirmation work, unlike overlapping advice-delay
sums. Unconfirmed publications remain censored.

## Prospective protocol

Saved before searches. Freeze clean source, a standard native release tune
build, compiler/config/overrides/log and executable stream hash. All lanes use
the existing twenty development positions (ten quiet, ten tactical), corpus
SHA-256 de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad.
This corpus is not independent playing-strength evidence.

Common controls: material evaluation, Hash 64, no book, MoveOverhead 0,
SpecTopN0, SearchMode=LazySMP, T_ASP_VOLATILITY=0, T_V2_SORT=0,
T_COOP_DRAIN=0 and T_COOP_PERSISTENT=0. Do not mix PGO, evaluator or other
search factors into this test.

1. Default-off identity: old frozen clean 1290743 tune engine (cb771a3a...)
   versus new engine, one worker, LazyFlags 15, depth 8, one per-position ABBA
   repeat, 80 searches. Require exact bestmove/score/PV/nodes/depth identity.
2. Mechanism: same new executable in both arms, exactly 2 workers,
   LazyFlags 16399, depth 10, one per-position ABBA repeat, 80 searches.
   Both arms set COOP_LATENCY=1; only COOP_PRIORITY changes 0/1. Require
   natural priority_reorders>0, valid root/outcome accounting and legal PVs
   before timing. Parallel signatures are observations, not identity gates.
   Keep all phase outcomes and unconfirmed publications, not just successes.
3. If that activity/correctness gate passes, run exactly 2 workers at 300 ms,
   two per-position ABBA repeats, 160 searches, same arms with COOP_LATENCY=0.
   No optional regular Lazy diagnostics. Require existing idle-host guard;
   permit at most 3 attempts only when earlier attempts block before any search,
   with at least 60 seconds between such resource-only retries. Keep all blocked
   manifests separate. A completed or interrupted run is never retried based
   on its depth, NPS or host-after outcome. Never stop unrelated processes or
   bypass the load guard. Pre/post snapshots are not continuous isolation.

A useful-depth screen advances only if timing is qualified, overall median
paired depth B-A is at least 0.5 ply, both A-first/B-first depth medians are
positive, and neither quiet nor tactical subgroup has a negative median.
These are development gates, not H1 or a strength verdict. NPS cannot rescue
a failed depth screen. This compares cooperative variants, not ordinary Lazy
SMP or YaneuraOu, and does not replace the fixed queued aspiration candidate.
Adoption still requires an independent H1 with the required owner manifests.

## Reproduction

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline <new-frozen-engine> --candidate <same-engine> \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output <new-directory> --threads 2 --count 20 --repeats 1 --depth 10 \
  --option LazyFlags=16399 --option T_COOP_LATENCY=1 \
  --option T_COOP_DRAIN=0 --option T_COOP_PERSISTENT=0 \
  --option T_ASP_VOLATILITY=0 --option T_V2_SORT=0 \
  --a-option T_COOP_PRIORITY=0 --b-option T_COOP_PRIORITY=1 \
  --source-ref <built-commit>
```

For counter-free timing use COOP_LATENCY=0, movetime 300 and repeats 2. Do not
add --diagnostics: cooperative results aggregate two participants, unlike the
independent-worker record contract. Replay every completed lane with the
frozen legal tool. No policy or threshold changes after observing this screen.
