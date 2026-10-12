# Lazy SMP helper-budget candidate (v0.4.0 base)

Status: rejected before SPRT. No playing default changed.

The earlier useful-work diagnostic called every node from a non-selected root
result “discarded root work.” That is an upper bound on final-result disposal,
not proof that the work was useless: sampled foreign-worker TT hits show that
some helper work is consumed by another worker.

This experiment changed one factor in a temporary build. Helper workers
received either 75% or 50% of worker 0's time, soft-time, and node budgets.
Worker 0 kept the full budget and remained responsible for stopping the group.
The option was removed after both screens failed their advancement contract.

## Screening contract

1. Compare the default and candidate on the same Sekirei-generated positions with
   material evaluation, `UseBook=false`, `Hash=64`, four workers, and 300 ms.
2. Require lower non-selected root nodes without reducing median completed
   depth. Counter-enabled runs are diagnostics, not NPS baselines.
3. Only if the diagnostic passes, preregister a colour-reversed material-only
   SPRT. Do not pool this screen with earlier vote or depth-ladder games.

The paired match, not the discarded-node percentage, decides whether the
candidate can advance. External evaluators and Claude's self-NNUE lane are out
of scope.

## Results

On 20 positions with two repetitions and counters disabled, the 75% helper
budget reduced discarded root work only from 75.0% to 73.5%. Median completed
depth stayed at 15, while repeated-bestmove stability fell from 85% to 75%.
That signal was too small and noisy to justify a strength match, so it was not
sent to SPRT or adopted.

The 50% helper budget reduced discarded root work from 75.0% to 61.2% on the
same 20-position, two-repeat screen. It also reduced median completed depth
from 15 to 14. Bestmove stability was 85% in both lanes. This failed the
depth-preservation requirement, so it too was rejected before SPRT. The result
shows that the helpers' shared-TT contribution cannot be treated as useless
merely because their final root result is not selected.
