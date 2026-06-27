# Sekirei

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)

[日本語](README_ja.md)

Sekirei is an experimental Rust shogi engine exploring speculative parallel search
and NNUE-style evaluation. It can play on floodgate/CSA and via USI, but its
strength, time management, and evaluation quality are still under active development.

The project is motivated by how Rust's ownership model enables safe concurrent search
— speculative parallel search with instant-cancel via atomics, without unsafe code.

## Current Status

- USI-compatible; works with ShogiGUI and similar GUIs
- CSA client for floodgate (account set via `FLOODGATE_ACCOUNT` in `.env`)
- NNUE-style evaluation available; weights not bundled — train from CSA data or use material fallback
- Floodgate rating is volatile (active testing)

## Principles

- **Zero `unsafe` in core logic** — all concurrency is handled by Rust's type system, atomics, and safe primitives
- **100% Pure Rust** — no C++ wrappers or FFI in the search/eval path

## Architecture

```
crates/
  sekirei-core/   — engine library
    board.rs      — position representation + do_move/undo_move/do_null_move
    movegen.rs    — legal move generation (generate_legal_moves, generate_legal_captures)
    search.rs     — YBW parallel alpha-beta + common search optimizations
    eval.rs       — NNUE evaluation (material fallback when weights not loaded)
    nnue.rs       — NNUE accumulator (incremental, SIMD-friendly, runtime weight loading)
    tt.rs         — lock-free transposition table (XOR-trick, depth-preferred)
    speculative.rs — preemptive speculation + RAII cancel
    policy.rs     — lightweight move scorer for speculation
  sekirei-usi/          — USI server → binary: sekirei
  sekirei-csa/          — floodgate CSA client → binary: sekirei-csa
  sekirei-match-runner/ — USI-vs-USI strength test manager → binary: sekirei-match
  sekirei-train/        — NNUE training pipeline (CSA parser, Adam SGD, weight I/O)
  sekirei-bench/        — microbenchmarks (movegen, perft, search, evaluate)
```

## Search (currently includes)

| Technique | Status |
|-----------|--------|
| Alpha-Beta (Negamax) | yes |
| PVS + YBW Parallel Search (rayon) | yes |
| Iterative Deepening | yes |
| Lock-Free TT (depth-preferred) | yes |
| Quiescence Search + Delta Pruning | yes |
| Killer Move Heuristic (2/ply) | yes |
| History Heuristic | yes |
| Aspiration Window | yes |
| Late Move Reduction (LMR) | yes |
| Null Move Pruning (R=3) | yes |
| Reverse Futility Pruning (depth ≤ 3) | yes |
| Futility Pruning (depth 1) | yes |
| Late Move Pruning (depth ≤ 2) | yes |
| Speculative Preemptive Search | yes |
| NNUE Evaluation | load via `cargo run -p sekirei -- weights.bin` |

## Roadmap

| Phase | Goal | Status |
|-------|------|--------|
| 1 | Foundation: Bitboard MoveGen, do/undo move, Perft | Complete |
| 2 | Lock-Free TT & YBW Parallel Search | Complete |
| 2.5 | Search Optimization (killer, history, LMR, NMP, RFP, futility, LMP) | Complete |
| 3 | Speculative Engine (preemptive spawning, RAII cancel) | Complete |
| 4 | NNUE Integration (weight I/O, eval wiring, training pipeline) | Complete |
| 5 | Protocol & Competition (CSA/floodgate, match runner) | In progress |

## Building & Running

```bash
# Development build
cargo build

# Optimized build (uses target-cpu=native via .cargo/config.toml)
cargo build --release

# Tests
cargo test

# Benchmarks
cargo bench --bench movegen

# USI engine (material eval fallback)
cargo run --release -p sekirei

# USI engine with NNUE weights
cargo run --release -p sekirei -- weights.bin

# Connect to floodgate (CSA)
cargo run --release -p sekirei-csa -- --user <name> --password <pass> --loop

# Strength test: sekirei vs sekirei (10 games, 1 sec byoyomi)
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 ./target/release/sekirei \
  --games 10 --byoyomi 1000

# Strength test: sekirei vs external USI engine
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 /path/to/suisho5 \
  --games 100 --byoyomi 10000

# Train NNUE from floodgate CSA files
# Download data from http://wdoor.c.u-tokyo.ac.jp/shogi/
cargo run --release -p sekirei-train -- --games /path/to/csa_dir --output weights.bin --epochs 3
```

## Benchmarks

Measured on Apple M4 Pro (`cargo build --release`, `target-cpu=native`).

| Metric | Value |
|--------|-------|
| Legal move generation (startpos) | ~5.5 µs / call |
| NNUE evaluate (startpos) | ~18.7 ns / call |
| Search depth 4 (startpos) | ~3.6 ms |
| Search NPS with NNUE (10 s time control) | ~1.1M nps, depth 13 |
| Test suite | 15 tests pass |

floodgate status: active testing; rating is currently volatile.

## Current Limitations

- NNUE weights are not bundled; train from floodgate CSA data or use the material eval fallback
- `setoption EvalFile` supported; in-game weight reload requires engine restart
- No pondering support

## Name Origin

**Sekirei** (セキレイ) is the Japanese word for wagtail — a small, nimble bird
known for its quick, darting movement.

The name reflects the engine's focus on fast, speculative search: committing to
moves early, then correcting course as the tree develops.
