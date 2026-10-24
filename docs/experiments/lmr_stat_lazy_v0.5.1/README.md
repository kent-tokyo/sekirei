# Lazy LMR history calculation (default OFF)

The `lmr_stat_lazy` core/USI feature skips the move-history score when
`depth < 2` or `move_count <= 1`. The only consumer is inside the opposite
LMR condition. Values that are consumed retain their original expression,
after singular-search depth adjustment and before playing the move.
Board and history reads must not move after the move is played.

`V2_LMR_STAT_LAZY=0` is the default; the `tune` feature exposes the runtime
switch for isolated correctness tests. The native Cargo feature fixes it ON
without tuning atomics. Existing pruning, move order, history updates,
evaluators and weights are unchanged. The test observer is absent from
normal native builds.

Tests cover lazy callback execution at depth/count boundaries, actual
search-path activation, legal PV replay and board restoration. An isolated
test compares material and an in-memory synthetic HalfKP evaluator at
6 rule-generated roots and depths 1, 2, 3 and 6. These are correctness
checks, not timings or training/model qualification.

```sh
cargo test -p sekirei-core --features lmr_stat_lazy --lib lmr_stat_lazy
cargo test -p sekirei-core --features tune --test lmr_stat_lazy -- \
  --ignored --nocapture --test-threads=1
cargo test -p sekirei-core --features tune --lib \
  search::v2::tests::lmr_stat_lazy_search_reaches_skipped_and_consumed_paths -- \
  --ignored --exact --nocapture --test-threads=1
```

Speed is not yet qualified. A single preregistered independent own-r3
runtime study uses current-source native OFF/ON builds (no PGO/tune/counters),
fresh material-generated 32 roots, exact 1T depth-12 ABBA and then 8T ABBA,
Hash64 MiB and 1000ms. Whole host-clean quartets only; at least 24 eligible
positions, 700ms minimum per arm and a 5000-draw bootstrap lower95% bound
above1 are required. Bootstrap seed20261025; no partial statistics or
mixed-candidate results. The SEE feature remains OFF in both arms.
No automatic default adoption, external-evaluator selection or strength claim.
