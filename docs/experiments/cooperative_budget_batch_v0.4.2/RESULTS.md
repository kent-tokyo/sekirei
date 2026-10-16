# Cooperative budget batching: unqualified; timing not started

`COOP_BUDGET_BATCH` remains **0 by default**. The implementation and local
contracts passed, but the one-shot development run closed at a diagnostic
validator error. No timed screen, independent H1, adoption, NNUE change or
YaneuraOu superiority claim follows.

## Frozen implementation

- Base: `cd77f387fdaed25aa3667d7664c104e9681f5cce`.
- Source: `4b4f2ce1fa095e2237ba4741052fe8618620b6c7`.
- Source tree: `70146fcd7c27e911ba2978d1ef7d1673b88a0c6e`.
- Same native tune engine on both arms, SHA-256
  `602739336f20db15ce6916f510b4585bbed0f71dea5960f0383130b29d233d21`.
- Material evaluation, Hash64, LazyFlags16399, COOP_MULTI1 and PERSISTENT1.
  Only `T_COOP_BUDGET_BATCH=0/1` differs.

Eligible timed, uncapped direct-parent children report nodes in batches of 512.
Local nodes and per-node stop checks remain exact. End guards settle before
main accounting or helper notifications, including panic and unstarted jobs.
The parent clock is checked every 512 local nodes and at job end, instead of
the old combined-node 4096 cadence. The configured deadline is unchanged.
Non-tune builds exclude the batching fields and hot branch. The combined
cooperative policy still has no independent H1 qualification.

Default core: 449 passed, 9 ignored. Tune core: 458 passed, 26 ignored,
including ten new unit tests. The isolated lifecycle test, workspace and tune
Clippy, formatting, 22 supplemented protocol tests, 11 archive tests and
16 integration-audit tests passed. The unit/integration contracts cover node
caps, nesting, stop/deadline boundaries, retained Arcs, panic, unstarted work,
notification order and exact final accounting.

The separate optimized cfg(test) natural diagnostic completed forty cases
once. Logical parent-node reports were observed as follows:

| Diagnostic only | 4T | 8T |
|---|---:|---:|
| Batched child nodes | 16,442,979 | 16,345,849 |
| Parent reports | 33,826 | 34,650 |
| Legacy-equivalent reports avoided | 16,409,153 | 16,311,199 |

These are instrumented logical report counts, not all shared atomic operations,
native speed, depth gain or Elo. Each case also checked reported child nodes
against cooperative main/helper accounting. The private observer is absent
from the native engine.

## Recording supplement and closed native run

The original [prospective README](README.md) is unchanged. The
[pre-native amendment](AMENDMENT.md) preserves a failed outer recording wrapper,
corrects only the schema in a separate receipt and classifies one mistakenly
frozen stdout stream as output. All forty-five actual inputs and all original
records remain unchanged. No natural diagnostic was repeated; the supplement
and new helpers were frozen before the first native search.

The A/A 1T and A/B 1T depth-8 lanes completed 80 rows each. All forty ABBA blocks
matched bestmove, score, PV, nodes and depth exactly. The diagnostic 2T/4T/8T
lane completed all 240 raw rows, then its validator rejected the census.

The USI `lazy_smp` agreement denominator is the number of returned root results
(`worker_stats.len()`, one in cooperative mode), not the requested participant
count. The frozen validator incorrectly required that wire field to equal
2/4/8. All 240 rows contained one worker-0 result and separate cooperative
helper accounting. Post-close source/raw inspection explains the error; it
does **not** restore policy-lane qualification or reopen the study.

All 400 rows and all 4,175 PV moves passed the frozen same-rules replay with
zero violations. This is not an independent rules oracle. The collector marks
only the first 160 rows fully validated; the 240-row policy lane remains
`complete_unvalidated`. Its `legal_pv_moves=1192` counts those fully validated
lanes, while the independent replay artifacts retain all 4,175 checked moves.

Prequiet and the 320-row native timing lane were never started. The collector's
`timed_status=prequiet_unqualified` means the run stopped before that stage,
not a measured busy-host failure. No completion-depth or NPS screen, retry,
pooled restart or SPRT was performed. Keep all default/H1/strength flags false.

## Durable evidence

[results.json](results.json) retains all 101 original files, including the
failed wrapper and policy validator output. The 106-file archive adds results,
standalone verifier, snapshot library and full source tar/raw Git commit:

`~/.local/state/sekirei/search-experiments/cooperative-budget-batch-v0.4.2/4b4f2ce1fa095e2237ba4741052fe8618620b6c7/`

Manifest SHA-256:
`902cb5327af9df5b21b7059a4c7ae1cecf4d0055346e6cb8fcd8625b6550b060`.
Offline hash/tree/commit verification passed with Git unavailable. The frozen
collector also recomputed the closed collection. Full raw USI GO transcripts
were not saved; parsed rows, driver logs and replay proofs were retained.
