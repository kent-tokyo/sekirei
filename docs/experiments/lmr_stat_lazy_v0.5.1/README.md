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

The 2026-10-25 native 8T study did **not** pass the speed gate. The feature
remains OFF. ON/OFF wall-NPS was **0.990565589** (about **0.94% slower**),
with a 5000-draw bootstrap 95% interval of **[0.983260174, 0.997138830]**.
This is a comparison against the same-source OFF build, not YaneuraOu.

[Qualification receipt](qualification.json) pins source `117abcdc9ee9`,
binaries, evaluator, corpus, protocol and local raw-evidence hashes. The
[32 CPU roots](cpu_roots32.sfen) were generated once with material evaluation,
selecting the first 16 per side without scores or timings. This corpus is
not an NNUE-owner holdout. Native builds used opt3, Thin LTO, one codegen unit,
panic-abort and target-cpu=native; no PGO, tuning or diagnostic counters.

The fresh 1T depth-12 ABBA passed all 128 calls: bestmove, score, PV, nodes,
completed depth and bound matched. The subsequent 8T ABBA completed all 32
quartets: 33 attempts, one whole-quartet host rejection, 128 accepted calls.
One position was excluded by the preregistered 700ms minimum, leaving 31.
The fixed gate required at least 24 eligible positions, point >=1 and lower
95% bound >1. Complete raw and bootstrap verification passed; the speed gate
failed. Hash64 MiB, 1000ms, own-r3 HalfKP/FV24 and options are in the receipt.

Two earlier wrapper failures are retained with null speed summaries. After
fixing a missing diagnostic argument and a host-admission race, both final
captures were run afresh. No old rows were pooled or rescued. All 260 PVs from the final captures,
including host-rejected calls, passed requested-root replay and an illegal-move
negative control. Source and input hashes matched before and after capture.

No default adoption, external-evaluator selection, YaneuraOu speed claim or
playing-strength claim follows from this result. Historical PGO results and
1T/4T/8T measurements remain separate. Local raw files stay under ignored
`results/lmr-stat-20261025`; their identities are retained in the receipt.
