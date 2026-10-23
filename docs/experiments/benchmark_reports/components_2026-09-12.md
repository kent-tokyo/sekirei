# Component benchmark repair and SFEN initialization pilot

## Outcome

The invalid six-ply do/undo fixture is repaired and checked before timing.
Initialization, board updates, NNUE work and output conversion are now separate
operations. The selected optimization removes two temporary `Vec<&str>`
collections from `Board::from_sfen`. Overfull ranks now return an error before
an invalid square can be constructed.

After this captured pilot, the working tree received separate root-search
stages, shared alpha-beta beta-cutoff bookkeeping, and a single-XOR update for
quiet-move derived bitboards. Those changes are covered by regression tests but
are intentionally outside the frozen before/after timings below; a new paired,
low-load capture is required before attributing any speed change to them.

SFEN initialization improved in each of three local paired runs. The median
of the three run medians was **7.512 us → 6.361 us (1.18x)**. This is a
**high-contention diagnostic, not a passed speed gate**: tail latency is large,
and unchanged controls also move substantially. It does not establish overall
search speed, an rsshogi ranking, or a playing-strength improvement.

## What was wrong

- The v6 six-ply roundtrip undid each move immediately, so the next move was
  applied to the wrong position/side. Both libraries' old row is invalid.
  v7 applies all six moves, observes the resulting state, then undoes in reverse.
- Generation previously included a raw-encoding checksum whose cost differed
  between `Move` and `Move32`. v7 observes the output slice instead. The old
  `legal_moves_packed_decode` diagnostic remains a combined decode/checksum
  operation; the new component decode row writes a separate reused output buffer.
- Legacy full-state rows included setup and, on Sekirei only, NNUE updates.
  Perft also has different leaf-counting and state-maintenance paths. These
  operations cannot establish a pure board-update or generic engine ranking.
- Historical Criterion default baselines could be overwritten by intervening
  experiments. The new capture runner preserves the executable, source hashes,
  raw samples and validated summary, and refuses to overwrite an output directory.

## Contract

- Base commit: `24664e14f9705b54d883b82d5c1d0c9786c05bae`; version stays `0.3.34`.
  Both arms have the same repaired harness. Candidate production change is in
  `board.rs`; tests changed in `lib.rs`. Capture-script changes are not in the
  measured executable. Captured harness/configuration hashes match between arms.
- Host: Apple M4, macOS 26.5.2, arm64; Rust 1.97.0 / LLVM 22.1.6.
- Build, for both arms:
  `CARGO_TARGET_DIR=/tmp/sekirei-target CARGO_INCREMENTAL=0 cargo build --offline --release -j 1 -p sekirei-bench --bin cross_library`.
  Repository settings: `target-cpu=native`, opt-level 3, thin LTO, one codegen unit.
- Reference: rsshogi 1.2.3 at `a1dbc020e0711574ba0bec6a7b123c411ccdd625`.
- Schema: `sekirei.component-benchmark.v1`; 21 samples per case, adaptive
  iteration count targeting 10ms per sample, rotating/reversing case order.
  Clock is wall time, not thread CPU time. Allocation/initialization is warmed
  before measurement; a scheduling stall during calibration can leave later
  samples shorter than the target. All raw durations/iteration counts are saved.
- Six-ply fixture: `7g7f 3c3d 2g2f 8c8d 2f2e 8d8e`; this is a quiet pawn
  sequence, not a capture/drop workload. Update timings exclude board construction
  and move generation. One sequence iteration includes six makes and six unmakes.
- Initializers measure Board/Position and output-buffer creation/destruction,
  not process startup, TT allocation or disk weight loading. Global tables and
  deterministic LCG weights are warmed by the preflight. NNUE is synthetic,
  not a trained evaluation file; the search-no-NNUE rows assert NNUE is inactive.
- Legal move sets (not just counts) match on startpos, the midgame-with-hands
  fixture and the drop-only fixture. Every sequence ply is checked for legality,
  SFEN equality and hash consistency. Undo restores hash, SFEN and accumulator.
- Main measurement order: before-1 / after-1 / after-2 / before-2 / before-3 /
  after-3 (AB, BA, AB). No Sekirei build/test ran during this sequence; other
  projects were active. The initial exploratory `/tmp` run is not aggregated.

Frozen binary SHA-256:

- Before: `0952100d050556872754caf6d450fe8d82f275aaf16f32789b6b1fa1531a6066`
- After: `3d1e6c479bcfa74c5dbe3ee6c2e8816fae8ecdb11f9d91166a0b9981f3436ebd`

## Results

Times below are **microseconds per whole iteration**, taking the median of
three run-level p50s (not a pooled percentile and not additive components of
one search). Raw p95s and samples are retained with each run.

| Sekirei operation | Work per iteration | Before | After |
|---|---|---:|---:|
| Direct startpos initialization | 1 board | 2.759 | 3.674 |
| SFEN startpos initialization | 1 board | 7.512 | 6.361 |
| Fixed output buffer creation/drop | 1 buffer | 0.715 | 0.759 |
| Board sequence, no NNUE | 6 roundtrips | 0.176 | 0.161 |
| Board sequence, with NNUE | 6 roundtrips | 0.999 | 1.093 |
| NNUE accumulator move/undo alone | 1 roundtrip | 0.150 | 0.180 |
| NNUE forward, startpos | 1 evaluation | 5.107 | 4.583 |
| Legal moves, startpos, reused Vec | 1 list | 0.447 | 0.486 |
| Raw encoding, reused output buffer | 30 moves | 0.067 | 0.057 |
| Packed decoding, reused output buffer | 30 moves | 0.067 | 0.073 |

The changed SFEN path by repeat (us):

| Pair | Before p50 | After p50 | Before p95 | After p95 |
|---|---:|---:|---:|---:|
| 1 | 6.255 | 5.255 | 16.386 | 18.970 |
| 2 | 7.512 | 6.833 | 20.760 | 16.975 |
| 3 | 8.616 | 6.361 | 18.622 | 28.890 |

Unchanged rsshogi controls (median of run p50s, us): SFEN initialization
3.949 → 3.841; startpos legal generation 0.362 → 0.347; six-ply state
roundtrip 0.735 → 0.831. Even these controls move. The generation and state
numbers must not be used to declare a winner: the state APIs maintain different
caches, and Sekirei's NNUE-enabled row has additional work absent from rsshogi.
An especially noisy unchanged Sekirei control, drop-only NNUE forward, moved
4.139 → 6.712 us. These variations preclude a firm 5% improvement claim.

SFEN initialization was the largest initial component; NNUE inference remains
another substantial cost. Removing parser allocations is a bounded improvement,
not an optimization of the recursive search hot path. No NNUE arithmetic,
move ordering or search pruning was changed.

## Evidence and reproduction

Local, gitignored artifacts are under
`results/component-benchmark-20260912/{before,after}-{1,2,3}/`:
`cross_library`, `provenance.json`, `samples.csv`, `validated_summary.json`.
These preserve all 22 component cases. The capture validator rejects missing or
duplicate samples, invalid timing, changed work units and mismatched percentiles.

```sh
python3 scripts/run_component_benchmark.py \
  --binary /tmp/sekirei-target/release/cross_library --output /tmp/new-component-capture
python3 scripts/run_component_benchmark.py \
  --replay results/component-benchmark-20260912/before-1 --output /tmp/new-before-replay
```

Replay retains the **original** source hashes and checks the frozen binary hash;
it does not label an old executable with the current working tree's provenance.
Building with matching flags before a fresh `--binary` capture remains the
caller's responsibility.

Validation: core unit tests **137 passed, 2 ignored**; benchmark regressions
**2 passed**; Python capture tests **3 passed**; scoped core/bench all-target
Clippy with warnings denied, workspace formatting, diff whitespace checks and
release metadata validation passed. Initial builds hit disk exhaustion; they
were retried after removing an unused 1.2GB temporary Cargo cache. Source,
models and game records were not deleted.

Next: repeat under low load, or add a portable thread-CPU clock before attempting
a formal speed gate. Keep the current result as diagnostic evidence only.
