# Janos

**JANOS** = *Jet-speed Ancestry Node Optimizer of Shogi*

Rust-based speculative shogi AI targeting world-class strength (Suisho / Hisui tier).

The core bet: Rust's ownership and type system make it possible to implement **hyper-aggressive speculative parallel search with safe instant-cancel** — something practically impossible to ship correctly in C++.

## Name Origin

The name pays tribute to three Hungarians named János, each embodying a defining quality of this project:

| Figure | Quality | Maps to |
|--------|---------|---------|
| **John von Neumann** (Margittai Neumann János) — founder of game theory | Precise, rigorous logic | Mathematically correct search tree |
| **Béla Bartók** (Bartók Béla Viktor János) — dismantled tradition, weaponized dissonance to forge a new musical language | Destruction of the old paradigm, creation of the new | Breaking C++'s dominance via safe Rust |
| **Háry János** (hero of Kodály's opera) — the baron of tall tales, boundless in imagination and daring | Bold, beyond-common-sense ambition | Speculative preemptive search: betting on moves before knowing they're right |

> 「緻密なロジック」「既存パラダイムの破壊と創造」「常識を超える大胆な大局観（投機的先読み）」

## Principles

- **Zero `unsafe` in core logic** — all concurrency is handled by Rust's type system, atomics, and safe primitives
- **100% Pure Rust** — no C++ wrappers or FFI in the search/eval path

## Architecture

```
crates/
  shogi-core/   — engine library
    board.rs    — position representation + do_move/undo_move/do_null_move
    movegen.rs  — legal move generation (generate_legal_moves, generate_legal_captures)
    search.rs   — YBW parallel alpha-beta + full search optimization suite
    eval.rs     — NNUE evaluation (material fallback when weights not loaded)
    nnue.rs     — NNUE accumulator (incremental, SIMD-friendly, runtime weight loading)
    tt.rs       — lock-free transposition table (XOR-trick, depth-preferred)
    speculative.rs — preemptive speculation + RAII cancel
    policy.rs   — lightweight move scorer for speculation
  usi/          — USI server → binary: janos
  csa/          — floodgate CSA client → binary: janos-csa
  match-runner/ — USI-vs-USI strength test manager → binary: janos-match
  train/        — NNUE training pipeline (CSA parser, Adam SGD, weight I/O)
  bench/        — microbenchmarks (movegen, perft, search, evaluate)
```

## Search Features

| Technique | Status |
|-----------|--------|
| Alpha-Beta (Negamax) | ✅ |
| PVS + YBW Parallel Search (rayon) | ✅ |
| Iterative Deepening | ✅ |
| Lock-Free TT (depth-preferred) | ✅ |
| Quiescence Search + Delta Pruning | ✅ |
| Killer Move Heuristic (2/ply) | ✅ |
| History Heuristic | ✅ |
| Aspiration Window | ✅ |
| Late Move Reduction (LMR) | ✅ |
| Null Move Pruning (R=3) | ✅ |
| Reverse Futility Pruning (depth ≤ 3) | ✅ |
| Futility Pruning (depth 1) | ✅ |
| Late Move Pruning (depth ≤ 2) | ✅ |
| Speculative Preemptive Search | ✅ |
| NNUE Evaluation | ✅ load via `cargo run -p usi -- weights.bin` |

## Roadmap

| Phase | Goal | Status |
|-------|------|--------|
| 1 | Foundation: Bitboard MoveGen, do/undo move, Perft | ✅ Complete |
| 2 | Lock-Free TT & YBW Parallel Search | ✅ Complete |
| 2.5 | Search Optimization (killer, history, LMR, NMP, RFP, futility, LMP) | ✅ Complete |
| 3 | Speculative Engine (preemptive spawning, RAII cancel) | ✅ Complete |
| 4 | NNUE Integration (weight I/O, eval wiring, training pipeline) | ✅ Complete |
| 5 | Protocol & Competition (CSA/floodgate, match runner, release) | ✅ Complete |

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
cargo run --release -p usi

# USI engine with NNUE weights
cargo run --release -p usi -- weights.bin

# Connect to floodgate (CSA)
cargo run --release -p csa -- --user <name> --password <pass> --loop

# Strength test: janos vs janos (10 games, 1 sec byoyomi)
cargo run --release -p match-runner -- \
  --engine1 ./target/release/janos \
  --engine2 ./target/release/janos \
  --games 10 --byoyomi 1000

# Strength test: janos vs external USI engine
cargo run --release -p match-runner -- \
  --engine1 ./target/release/janos \
  --engine2 /path/to/suisho5 \
  --games 100 --byoyomi 10000

# Train NNUE from floodgate CSA files
# Download data from http://wdoor.c.u-tokyo.ac.jp/shogi/
cargo run --release -p train -- --games /path/to/csa_dir --output weights.bin --epochs 3
```

## Agent Roles

See [AGENTS.md](AGENTS.md) for the multi-agent collaboration model used to develop this codebase.

## Task Tracking

See [tasks/todo.md](tasks/todo.md) for the full milestone checklist.  
See [tasks/lessons.md](tasks/lessons.md) for architectural lessons learned.
