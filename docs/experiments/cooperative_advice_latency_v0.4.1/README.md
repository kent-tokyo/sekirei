# Cooperative helper advice latency diagnostics

This is opt-in observation, not a new search policy or a playing-strength
candidate. No NNUE training, weights, labels or hold-out data are used.
T_COOP_LATENCY defaults to zero and is independent of T_COOP_DRAIN and
T_COOP_PERSISTENT. It only observes the existing two-worker cooperative root
prototype (SearchMode=LazySMP, Threads=2, LazyFlags=16399).

## Measurement contract

Each enabled generation holds a monotonic clock, the private worker-0 child
Budget, activity epoch, and metadata slots separate from ordinary candidate
slots. Disabled generations allocate no observer or telemetry slots. There
are no per-node observer writes and no new scout cancellation or score use.
Existing pair/triple search methods and CooperativeRootStats stay compatible;
the non-exhaustive wait snapshot gains an optional non-exhaustive latency
snapshot. USI emits a separate cooperative_root_latency line when enabled.

Main root child calls are labelled owned scout, owned re-search or helper
confirmation. Other phases are idle. An epoch distinguishes same-phase ABA.
The helper samples phase before/after a single time/node sample just before
candidate publication; a changed epoch is classified as a transition. These
are observations near publication, not a perfectly simultaneous snapshot.
Metadata is stored before the existing slot-ready Release and acquired with
the candidate. Only a valid generation/depth candidate actually starting its
main confirmation contributes a latency/node-delta sample.

Publication-to-confirmation node progress uses worker 0's private Budget.
Helper nodes and the candidate's own confirmation work are excluded. Several
pending candidates can have overlapping intervals, so summed node deltas are
not unique discarded work, a waste percentage or predicted savings. Report
publication and confirmation counts, unconfirmed publications, sum/max delays
and phase buckets together. Unconfirmed advice is censored, not assigned zero
latency. Rejected publications account for cancellation races after a helper
fail high; they are not successful advice.

Beta-scout advice means the helper's reported lower bound is at least the
fixed aspiration-pass hi, sampled during an owned scout. It must still pass
worker-0 full-depth verification. Its count and eventual confirmed cuts do
not justify adopting the helper score or interrupting a main child by themselves.
No policy or parameter adoption is part of these diagnostics.

## Prospective protocol

Saved before any new diagnostic searches. Freeze a clean source commit,
standard native release tune build, compiler/config/build log and executable
stream hash. Do not reuse a PGO profile from another source revision. Both
lanes use material evaluation, Hash=64, no book, SpecTopN=0, MoveOverhead=0,
T_ASP_VOLATILITY=0, T_COOP_DRAIN=0 and T_COOP_PERSISTENT=0.

Use the existing twenty-position diagnostic corpus (ten quiet, ten tactical),
SHA-256 de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad.
It is not an independent strength corpus and cannot be promoted by changing
its seed or label. No position selection after observing these counters.

1. Default-off identity: old clean frozen tune build at 62573f9 versus new
   build, one worker, depth 8, LazyFlags=15, one per-position ABBA repeat
   (80 searches). Require exact bestmove/score/PV/nodes/depth identity.
2. Mechanism: same new executable in both arms, two workers, depth 10,
   LazyFlags=16399, one per-position ABBA repeat (80 searches). Only
   T_COOP_LATENCY=0/1 changes. Use intrinsic root counters without optional
   regular Lazy diagnostics; their independent-worker contract remains strict.
   Record all publication phases, rejected/unconfirmed advice, private-main
   node progress, latency and beta-scout conversion, preserving quiet/tactical
   subgroups. Parallel signature changes are observations, not benefits.
3. Replay all saved PVs through the frozen legal-move tool and validate root
   attribution and telemetry arithmetic on every row. This is a same-rules
   check, not an independent rule oracle or mate-score proof.

No fixed-time or NPS qualification is planned: the enabled observer adds cost
and can change scheduling. All instrumented timing is explicitly unqualified,
even without the regular --diagnostics option. Diagnostic lanes may overlap;
fixed-depth wall times cannot be compared as speed evidence. No 1T/4T depth
boost, Elo, scout-preemption benefit or YaneuraOu superiority follows from it.
The already queued aspiration strength candidate remains unchanged and still
requires owner-attested identity exclusion manifests for its independent gate.

## Reproduction

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline <new-frozen-engine> --candidate <same-engine> \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output <new-directory> --threads 2 --count 20 --repeats 1 --depth 10 \
  --option LazyFlags=16399 --option T_COOP_PERSISTENT=0 \
  --option T_COOP_DRAIN=0 --option T_ASP_VOLATILITY=0 \
  --option MoveOverhead=0 --a-option T_COOP_LATENCY=0 \
  --b-option T_COOP_LATENCY=1 --source-ref <built-commit>
```

Do not add --diagnostics to this coordinated prototype lane. Results and
interpretations will be recorded separately from this prospective plan.
