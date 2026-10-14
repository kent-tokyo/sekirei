# V2 null-move diagnostics and ancestor experiment

Status: implementation candidate; no strength result or adoption claim. NNUE,
training data and the normal search parameters are unchanged. `V2_NMP_PRIOR=0`
is the default. `V2_EAGER_LOSING_SORT=1` provides the old eager-sort timing for
an otherwise identical, frozen tune build.

## Diagnostics first

Call `SearchDiagnostics::counters_only()` and attach it to the searcher, then
read `SearchDiagnostics::nmp_snapshot()`. These atomics and exact-position
tracking run only with per-node diagnostics enabled. Production root-safety
observers do not enable them. Run timing and playing-strength gates without
these counters because instrumentation changes throughput.

`NmpSnapshot::by_depth` has 32 rows; row 31 includes depths 31 and above. Its
seven columns, also available as named totals, are:

| Column | Counter | Meaning |
| --- | --- | --- |
| 0 | `attempts` | Null search started; a later budget abort remains an attempt. |
| 1 | `fail_high` | Completed null search scored at least beta, including decisive scores that the search refuses to prune on. |
| 2 | `verification_attempts` | Verification search started after a usable null fail-high. |
| 3 | `verification_successes` | Completed verification retained the fail-high. |
| 4 | `verification_overturns` | Completed verification fell below beta. |
| 5 | `pv_returns` | Same worker later searched the exact position hash as a PV node after a completed null search. |
| 6 | `tracking_overflow` | Completed attempt omitted from bounded PV-return tracking. |

A PV return means a PV search call, not membership in the final reported root
PV. Track the most recent attempted depth for each hash and consume that entry
when its PV return is counted. The per-worker exact table holds at most 16,384
positions and resets for every new invocation's budget, including direct API
calls that do not advance the TT generation. Its overflow is explicit; the other
counters do not sample. A verification aborted by the budget appears only in
`verification_attempts`, never as a success or overturn.

Report fail-high/attempts, successes/completed verifications and
turnovers/completed verifications by depth. Keep aborted verifications and
tracking overflow visible. Sparse or high-overturn regions are evidence
against strengthening the null reduction.

## Isolated opt-in factor

`T_V2_NMP_PRIOR=1` in a build with `--features tune` adds at most one reduction
ply after a strong fail-high at an active same-side ancestor two or four plies
above. A fail-high at least 200 centipawns above beta increments a saturating
counter (maximum 3). A completed verification overturn decrements it.

Each frame binds its evidence to its exact position hash. Entering a different
sibling position clears the evidence. Accepted cutoffs have no descendants;
ancestral evidence becomes usable when a previously probed node is revisited
or researched and then searches real children. The correction is disabled in
check, at depths below 6, for decisive evaluation/beta values, with fewer than
12 on-board pieces, or when the moving side has only king/pawn material.
These conservative exclusions do not prove the absence of zugzwang.
`NmpSnapshot::prior_reductions` counts actual uses so an inactive experiment
cannot be mistaken for evidence of safety or strength.

Do not combine this factor with helper LMR, root hints, retirement, root split
or another tuning factor in its strength test. First collect baseline NMP
statistics, then compare this one factor with fixed corpus, binary, hash size,
material-only evaluation and Sekirei-generated openings. No external evaluator
output may select reductions or constants. Require the predeclared H1 gate
before adopting the option or making a strength claim.

## Validation

Focused tests cover depth buckets and totals, bounded exact-position tracking,
once-only PV-return consumption, frame reset between sibling positions, and
all conservative one-ply exclusions. A controlled real null search covers the
fail-high and verification counters; the isolated prior regression requires an
actual one-ply correction. A fixed-depth observed/unobserved test
requires best move, score, completed depth, node count and PV to match and
checks root-board/accumulator restoration.

Run the focused suite with:

```sh
cargo test -p sekirei-core --lib search::v2::tests -- --test-threads=1
cargo test -p sekirei-core --features tune --lib tuned_nmp_prior_restores_board_and_returns_legal_move -- --ignored --exact search::v2::tests::tuned_nmp_prior_restores_board_and_returns_legal_move --test-threads=1 --nocapture
```

The losing-tactical timing comparison is a separate factor. Require exact
single-thread fixed-depth best move, score, PV and nodes, then measure repeated
counter-free 1T/4T ABBA timing across quiet and tactical positions. Shared-TT
4T searches have schedule-dependent nodes, so report their distributions
rather than demand deterministic multithread node equality.

## v0.4.1 counter screen (2026-10-14)

A preregistered, separate screen used the clean `4344202` release/tune engine
(SHA-256 `18b427475993da42ed0c2c411b03e8785e8fe6b1c4f03bfd046ff4dc2b0e08a2`),
whose source tree equals merged main `3c40d00`. The existing 20-position
quiet/tactical diagnostic corpus was unchanged. One and four workers each
ran per-position ABBA, one repeat, 300 ms, Hash=64, MoveOverhead=0,
LazyFlags=15, material evaluation and no book. Only T_V2_NMP_PRIOR=0/1 differed.
Optional counters were enabled; compilation elsewhere was observed. This is
not qualified timing, an independent corpus, or a strength gate.

| Workers | Arm | Attempts | Completed verifications | Overturns | Actual corrections |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | OFF | 167,366 | 1,606 | 22 | 0 |
| 1 | ON | 168,841 | 1,622 | 22 | 0 |
| 4 | OFF | 557,536 | 2,545 | 62 | 0 |
| 4 | ON | 589,296 | 3,048 | 68 | 0 |

All 160 searches completed; no corrections occurred in any of the 80 ON
searches. Tracking overflow and unfinished verifications were zero. The
frozen rules tool replayed all 1,316 PV moves with zero bestmove/PV-prefix or
legal-transition violations. Counters and depth histograms are in
[screen-v0.4.1.json](screen-v0.4.1.json), with protocol and raw-artifact hashes.

This sample does not exercise the active correction and cannot establish its
safety or benefit. Fixed-time counts differ even with inactive pruning due to
scheduling and execution overhead. Do not interpret count, depth or verification
rate differences as a causal effect. Do not loosen the activation rules to fit
this corpus. The option remains disabled and does not advance to SPRT; the
independently queued aspiration candidate is unchanged.
