# Quiet continuation-row scoring

`V2_CONT_ROWS` remains **0 by default**. No speed improvement or strength claim.

The candidate selects continuation-history row references once per nonempty
quiet-scoring batch, after the dead-quiet shortcut. Live values are added in
1/2/3/4/6-ply order. Short mode selects only 1/2; missing predecessors remain
zero. References expire before child search or history updates. The default
path retains the original indexed lookup. Existing public API signatures, Board/Frame/Entry layouts, move ordering
keys, pruning rules and training/evaluator behavior remain unchanged.

Both variants passed 420 ordinary core tests (six ignored), isolated real
mechanism checks, material72 + own random11 HalfKP72 fixed-depth signatures
including nodes/PV/bounds and root restoration, 10M legal-move visits, fmt,
and release default/tune clippy. Each native pair also passed 31 exact 1T
pairs, four isolated 4T worker-zero pairs and two 4T lifecycle arm cases
(72 fresh search processes). Ordinary tests were unoptimized; the isolated
checks used the recorded Cargo release profile. Timing is separate.

| Variant | Independent own 4T result | Decision |
| --- | --- | --- |
| R1: five-row iterator | 128 accepted calls / 32 eligible positions; wall NPS ratio 0.991734, 95% CI [0.975483, 1.006736] | Gate rejected |
| R2: five explicit additions | Last attempt stopped at 96/128 accepted calls after three host-rejected quartets at one position | Incomplete, unqualified |

R2's two earlier attempts also remained unqualified: the first stopped with
zero searches on ENOSPC; the second had zero accepted calls and one
host-rejected quartet, then a host-monitor OSError while disk space was
below the minimum. Its original report retained a stale `running` status
because the final update failed; the process exited120. A separate receipt
preserves that fact. No partial NPS statistics were calculated or pooled.
R2's final attempt made140 actual calls across35 quartets;96 were accepted.

Own H1 conditions: Threads/Rayon4, Hash64, own synthetic HalfKP/FV16,
Auto, SpecTopN0, LazyFlags15, MoveOverhead0, diagnostics off, cold fresh
processes and one-second per-position ABBA. R1 used fresh own32 seed202610166;
R2 used seed202610167 and excluded R1 as well as development and earlier
corpora. The preregistered gate requires128 accepted calls, at least eight
eligible positions, and a bootstrap95% lower bound above1. Host/disk guards
were retained. Neither candidate qualified for private YaneuraOu testing.

Native ASM verified frame/table/index work moved outside the quiet loop.
R2 also removed R1's descriptor loop; baseline/R2 node stack reservation
was2768/2736bytes. These are compiler observations, not speed evidence.

Raw logs, source snapshots, frozen binaries and binding proofs remain in
separate experiment worktrees; `results.json` records their hashes and paths.
Only verified-unused regenerable compiler caches were removed, with
protection hashes checked before/after. Measurement evidence was retained.
