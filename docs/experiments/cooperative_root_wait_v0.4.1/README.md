# Cooperative root candidate confirmation during helper wait

This opt-in candidate targets useful work while worker 0 is waiting for its
last helper probe. It is not a default policy or a playing-strength result.
No NNUE training, weights, labels or hold-out data are used.

## Contract

Build with `--features tune`. Set SearchMode=LazySMP, Threads=2,
LazyFlags=16399 and T_COOP_DRAIN=1. Default T_COOP_DRAIN=0 retains the
previous control flow. Other thread counts retain ordinary Lazy SMP.

After worker 0 exhausts the shared root queue, it confirms ready helper
candidates while awaiting helper completion. Each candidate is rechecked at
full depth against the current alpha; helper scores are never adopted directly.
Beta cutoff cancels the generation. A private capacity-one notification channel
coalesces wakeups with try_send; it is allocated only when enabled. A RAII guard
publishes helper completion on return or panic. Notifications are hints: scoped
join or persistent completion receive and the final candidate drain remain
mandatory. External stop is checked between receiver waits (1 ms timeout,
not a strict operating-system scheduling bound). Active main scouts are not
interrupted by incoming advice in this candidate.

The existing CooperativeRootStats layout and pair-returning search methods
remain available. The additive non-exhaustive CooperativeRootWaitStats and
search_cooperative_root_with_history_and_wait_stats API expose wait activity.
The Lazy SMP adapter publishes both snapshots at the same invocation boundary.
USI emits an additional `info string cooperative_root_wait` line only when
the two-worker cooperative path requested this candidate.

waiting_confirmations counts confirmations started before the helper finished;
waiting_confirmation_nodes and waiting_alpha_raises belong to those starts,
even if the helper finishes during confirmation. Confirmation nodes are part
of main_nodes, never extra total nodes. notification_wait_us measures receiver
blocking. join_wait_us now includes notification and completion/join blocking,
excluding confirmation work. Neither waiting time is automatically wasted work.
These root-level channels are safe Rust; no lock-free claim is made for them.

## Prospective diagnostic protocol

Saved before any diagnostic search of this candidate. Freeze a clean source
commit, release tune executable, build log, compiler/config and stream hash.
All searches use material evaluation, Hash=64, MoveOverhead=0, no book,
SpecTopN=0, T_ASP_VOLATILITY=0, T_COOP_PERSISTENT=0. The existing twenty-position
useful-depth corpus (ten quiet, ten tactical) is diagnostic only, SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.

1. Default-off regression: frozen prior clean engine versus new engine, one
   worker, depth 8, LazyFlags=15, one per-position ABBA repeat. Require exact
   bestmove/score/PV/nodes/depth identity. Replay all PV moves.
2. Mechanism: same new executable in both arms, exactly two workers,
   LazyFlags=16399, depth 10, one per-position ABBA repeat. Only T_COOP_DRAIN
   changes from 0 to 1. Intrinsic cooperative counters are parsed without
   optional Lazy diagnostics, whose per-independent-worker contract is retained.
   Record wait confirmations, nodes, alpha raises, root attribution and legal
   PVs. Parallel signatures are observations, not identity gates. Actual
   waiting_confirmations must be nonzero to establish natural activation.
3. If mechanism activates and correctness passes, use the same arms at 300 ms,
   two ABBA repeats. Require the existing idle-host guard; keep blocked attempts
   separate and never pool incomplete results. Compare paired completed depth,
   node work and elapsed time, retaining quiet/tactical subgroups. Do not change
   the factor or protocol in response to observed outcomes.

This compares two variants of the cooperative prototype. It does not establish
superiority over ordinary Lazy SMP or YaneuraOu. No adoption follows from NPS
or depth alone; independent H1 is required. The previously queued independent
aspiration candidate and its exclusion-manifest requirement remain unchanged.

## Reproduction

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline <new-frozen-engine> --candidate <same-engine> \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output <new-directory> --threads 2 --count 20 --repeats 1 --depth 10 \
  --option LazyFlags=16399 --option T_COOP_PERSISTENT=0 \
  --option T_ASP_VOLATILITY=0 --option MoveOverhead=0 \
  --a-option T_COOP_DRAIN=0 --b-option T_COOP_DRAIN=1 \
  --source-ref <built-commit>
```

Do not add --diagnostics to the cooperative lane: its result is one coordinated
aggregate, not two independent worker histories. Validate completed runs with
scripts/validate_paired_pv.py. Fixed-depth wall times are not qualified speed
measurements. Results will be recorded separately from this prospective plan.
