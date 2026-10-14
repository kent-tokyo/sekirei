# Empty root-candidate polling

The opt-in two-participant cooperative root split polls every candidate slot
before taking the next main root move and once after helper completion. Most
slots are empty, but each poll previously executed an atomic swap even when
there was no candidate.

Generation::take now checks ready with an Acquire load and returns on false.
A ready slot still uses the existing consuming Acquire swap and generation
check. The helper publishes the payload before a Release ready store. A late
publication is retained for the next poll or the final drain after the scoped
join or persistent completion receive. Beta/stop cancellation retains its
existing discard policy. No pruning, root order, candidate confirmation,
result selection, search parameter or evaluator changes.

A Barrier regression fixes helper publication after the first empty poll,
then checks that the final drain after join retrieves the complete payload
exactly once. All 22 root-split tests passed, including both executor modes,
legal PV/board restoration, shared budgets, stale generations, latest-alpha
confirmation, history isolation, cancellation and panic cleanup.

This removes a read-modify-write operation on the empty path. It does not
establish a measured speed, effective-depth or playing-strength improvement.
The cooperative root flag remains disabled by default; previously measured
executor results remain separate. This change is independent of the NMP-prior
counter screen, which used a frozen engine built before this source edit.
