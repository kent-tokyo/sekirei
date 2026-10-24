# Cross-library benchmark: v0.3.33 development snapshot

This is a local rules-library throughput diagnostic, not a playing-strength or
overall engine-speed claim. Schema v4 constructs positions outside the timed
region, reuses move buffers, alternates the measured-library order, and reports
the median plus the full range of seven samples.

## Environment

- Revision: `4c568b1` plus the uncommitted benchmark and move-generation changes
- OS/architecture: Darwin 25.5.0, `arm64`
- Rust: `rustc 1.97.0` (`aarch64-apple-darwin`, LLVM 22.1.6)
- Build: `SEKIREI_BENCH_ITERATIONS=10000 cargo run --release -p sekirei-bench --bin cross_library`
- Comparison: `rsshogi` 1.2.3 at `a1dbc020e0711574ba0bec6a7b123c411ccdd625`
- Fixtures: start position and a legal midgame with captured pieces in hand

## Results

Times are nanoseconds per iteration. Each range is minimum--maximum across the
seven samples.

| Operation | Library | Median | Range | Scope |
|---|---|---:|---:|---|
| Pseudo moves, startpos | Sekirei | 274 | 226--352 | setup excluded, reused `Vec` |
| Legal moves, startpos | Sekirei | 282 | 281--297 | paired, setup excluded, reused buffer |
| Legal moves, startpos | `rsshogi` | 203 | 201--205 | paired, setup excluded, reused `Move32List` |
| Perft(2), startpos | Sekirei | 6,455 | 6,447--6,465 | paired, Perft only |
| Perft(2), startpos | `rsshogi` | 8,008 | 7,697--8,685 | paired, Perft only |
| Perft(3), startpos | Sekirei | 192,399 | 192,230--192,682 | paired, Perft only |
| Perft(3), startpos | `rsshogi` | 250,672 | 248,235--252,096 | paired, Perft only |
| Legal moves, midgame with hands | Sekirei | 486 | 485--488 | paired, setup excluded, reused buffer |
| Legal moves, midgame with hands | `rsshogi` | 358 | 357--371 | paired, setup excluded, reused `Move32List` |
| Perft(2), midgame with hands | Sekirei | 31,900 | 31,776--31,946 | paired, Perft only |
| Perft(2), midgame with hands | `rsshogi` | 51,440 | 50,447--52,633 | paired, Perft only |
| Fixed six-move update | Sekirei | 1,442 | 1,441--1,449 | setup included; hash and NNUE updated |
| Fixed six-move update | `shogi_core` 0.1.5 | 181 | 180--182 | setup included; prevalidated moves |
| Legal-root state roundtrip | Sekirei | 4,924 | 4,919--4,943 | legal generation plus full do/undo |
| Legal-root state roundtrip | `rsshogi` | 3,856 | 3,827--3,864 | legal generation plus rules-state apply/undo |

Sekirei is 1.241x faster on start-position Perft(2), 1.303x faster on
start-position Perft(3), and 1.613x faster on the midgame-with-hands Perft(2)
fixture. `rsshogi` remains 1.39x faster on start-position legal generation,
1.36x faster on midgame legal generation, and 1.28x faster on the roundtrip
diagnostic. The immediate multi-ply Perft target is therefore met locally, but
Sekirei has not surpassed `rsshogi` across all measured primitives.

## Internal fixed-depth search check

The same machine also compared the current candidate against clean `4c568b1`
in a detached worktree with Criterion's `search_depth4_startpos` benchmark:

| Revision | Median | Estimate interval |
|---|---:|---:|
| Current candidate | 2.265 ms | 2.229--2.305 ms |
| Clean `4c568b1` | 3.348 ms | 3.317--3.379 ms |

The candidate is about 1.48x faster in this bounded fixed-depth search. This is
an internal before/after result, not an `rsshogi` search comparison.

A later search-only optimization reuses the caller's already-known check state
when building the legal-move constraints. Its first comparison against the
saved candidate baseline measured 2.213 ms (2.199--2.230 ms), a 2.27% median
reduction with p=0.01. Two absolute reruns ranged from 2.524 to 3.131 ms while
the desktop process was consuming CPU, so this remains a mechanism-level
signal inside Criterion's 5% noise threshold, not a separate speed claim.

The subsequent no-check/no-pin fast path keeps the opponent-king exclusion but
monomorphizes away pin/evasion branches. The 10,000x7 cross-library run above
measured 282 ns for start-position legal generation and 486 ns for the
midgame-with-hands fixture. A depth-4 Criterion sample measured 2.407 ms
(2.385--2.427 ms); because the saved Criterion baseline was load-sensitive,
this is recorded as a diagnostic sample rather than a clean before/after
search-speed claim.

The first strict schema-v2 baseline measured 671 ns for Sekirei start-position
legal generation and 575,447 ns for Perft(3). The current values are about 54%
and 62% lower. Retained changes include direct pin/check masking, static king
destination checks, count-only Perft leaves, direct drop counting with isolated
uchifuzume probes, and removal of intermediate pseudo-move buffers.

The legal-generation rows do not use identical container representations:
Sekirei fills its public `Vec<Move>` API, while `rsshogi` fills a fixed-capacity
`Move32List`. The roundtrip rows also differ because Sekirei's public
`do_move`/`undo_move` synchronizes its hash and NNUE accumulator, whereas the
`rsshogi` row exercises its rules-state `apply_move32`/`undo_move32` path.
`shogi_core` does not provide a legality checker or move generator, so its row
is only a lower-level state-update reference.

A safe, fully initialized fixed-capacity move-list prototype improved isolated
generation by about 3% at startpos and 12% with hands, but produced no
measurable fixed-depth search improvement over the pooled `Vec` path. It was
therefore removed rather than adding an abstraction that only improved the
microbenchmark.

## Validation

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --release`
- `cargo test -p sekirei-core --release tests::random_perft_mated_10m -- --ignored --exact`
  (10,000,000 deterministic random positions, 108.98 s, no mismatch)

## Reproduction

```sh
SEKIREI_BENCH_ITERATIONS=10000 \
  cargo run --release -p sekirei-bench --bin cross_library
```

Repeat on another controlled machine before making a portable speed claim.
Do not infer USI search speed, NNUE throughput, Elo, or playing strength from
these rules-library measurements.
