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

## NNUE Training

See `docs/training_lessons.md` for durable design notes (capacity collapse and its fix,
validation-split policy, teacher-search caching, checkpoint reproducibility fields) and
`docs/experiments/` for specific experiment records.

### From CSA files (standalone)

```bash
# Basic: train from floodgate CSA files (download from http://wdoor.c.u-tokyo.ac.jp/shogi/)
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin \
  --epochs 3 --quiet --min-ply 20 --min-rate 1800 --label-depth 4

# With a WDL (game-result) term blended into the teacher (CSA path only):
# teacher = λ·eval_teacher + (1-λ)·wdl_target, λ=0.7 is a reasonable starting
# point to sweep from. Positions from an aborted/timed-out/illegal-move game
# (GameResult::Unknown) fall back to eval-only automatically.
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin \
  --epochs 3 --quiet --min-ply 20 --min-rate 1800 --label-depth 4 --wdl-lambda 0.7

# Configurable LR schedule + held-out validation on the CSA path. --validation-ratio
# splits by game (leak-safe: every sampled position from one game lands on one
# side); cosine needs --min-lr since it decays toward that floor by the final
# epoch, not toward zero. See docs/training_lessons.md for why an unfloored
# step-half schedule can make an early-stopped checkpoint hard to interpret,
# and for the per-epoch diagnostics (active/saturation ratios, output std,
# update norm) this run also prints and records in the checkpoint .meta.json.
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin \
  --epochs 20 --wdl-lambda 0.7 --lr-schedule cosine --min-lr 0.00001 --warmup-epochs 1 \
  --validation-ratio 0.15 --seed 42
```

### With Quietset (stability-filtered)

[quietset](https://github.com/kent-tokyo/quietset) filters training positions by label stability across multiple search depths, reducing noisy teacher labels.

```bash
# 1. Export multi-depth observations
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --export observations.jsonl \
  --depths 2,4,6,8 --quiet --min-ply 20 --min-rate 1800

# 2. Score label stability
quietset score observations.jsonl > scored.jsonl

# 3a. Train with stable samples only (keep where stability >= 0.85)
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights_keep.bin \
  --scored scored.jsonl --min-stability 0.85 --epochs 3

# 3b. Or weight loss by stability_score (unstable positions contribute less)
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights_weighted.bin \
  --scored scored.jsonl --stability-weighted --epochs 3
```

### With shogiesa + quietset (official pipeline)

[shogiesa](https://github.com/kent-tokyo/shogiesa) extracts and labels positions;
[quietset](https://github.com/kent-tokyo/quietset) scores label stability.
sekirei-train accepts `positions.jsonl` directly via `--positions`, bypassing CSA parsing entirely.

The one-shot pipeline script handles all stages and runs an Elo gate at the end:

```bash
# Tier 1 — Quick (depths 2,4, ~hours)
bash scripts/train_with_shogiesa_quietset.sh data/csa weights_new.bin data/weights_v007.bin

# Tier 2 — Standard (depths 2,4,6)
DEPTHS=2,4,6 bash scripts/train_with_shogiesa_quietset.sh data/csa weights_new.bin data/weights_v007.bin

# Tier 3 — Deep: re-label borderline positions at depth 4,6,8 then retrain
# Step 1: score borderline positions at higher depth into a separate file
quietset select data/stage3/scored.jsonl --class borderline \
  | shogiesa label --engine ./target/release/sekirei --depths 4,6,8 \
  | quietset score --profile game-ai-single-engine \
  > data/stage3/deep_scored.jsonl
# Step 2: retrain with the deep labels merged in via EXTRA_SCORED
EXTRA_SCORED=data/stage3/deep_scored.jsonl \
DEPTHS=2,4,6 \
bash scripts/train_with_shogiesa_quietset.sh data/csa weights_deep.bin data/weights_v007.bin
```

The script saves intermediate files under `data/runs/<timestamp>/` (override with `RUN_DIR=...`)
and match results in `results/`. Each run also writes a `manifest.json` linking weights to their
training parameters. Available env overrides: `DEPTHS`, `GAMES`, `MIN_PLY`, `MAX_PLY`, `RUN_DIR`,
`EXTRA_SCORED`. To run stages manually:

`data/runs/` intermediate files (raw extracts, label observations, scored jsonl -- often
multi-GB) are pruned automatically at the start of each pipeline run by `scripts/cleanup_runs.sh`:
it only deletes a run's `stage1`/`stage2`/`stage3` once that run has a `manifest.json` (proven
complete), the run isn't referenced by name in any `scripts/*.sh` (a live cross-run dependency),
and it's older than `MIN_AGE_DAYS` (default 3). `checkpoints/`, `manifest.json`, and `pipeline.log`
are never touched. Run it manually with `bash scripts/cleanup_runs.sh` (dry run by default,
`APPLY=1` to actually delete).

```bash
# Stage 1: extract
shogiesa extract --input ./data/csa --out data/stage1/positions.jsonl \
  --min-ply 20 --max-ply 160 --every-n-plies 4 --dedup

# Stage 2: label
shogiesa label --input data/stage1/positions.jsonl \
  --engine ./target/release/sekirei --depths 2,4 --timeout-ms 10000 \
  --out data/stage2/observations.jsonl

# Stage 3: score
quietset score data/stage2/observations.jsonl --profile game-ai > data/stage3/scored.jsonl

# Train
cargo run --release -p sekirei-train -- \
  --positions data/stage1/positions.jsonl \
  --scored data/stage3/scored.jsonl \
  --stability-weighted --validation-ratio 0.1 \
  --checkpoint-dir data/checkpoints \
  --output data/weights_new.bin
```

## Strength Regression

To verify that a change actually improves play strength, run a match against a known baseline and
apply the Elo gate. Every weight change should clear the gate before being promoted.

A `startpos`-only match between two deterministic engines can collapse into a handful of games
replayed hundreds of times (TT/thread-count no longer add variation) — 400 such "games" can carry
the statistical power of ~40 real trials. `strength_regression.sh` therefore requires
`--positions` by default (`data/gate/openings_standard.sfen`, 99 SFEN positions plus a header
comment × `--games-per-position` for real opening diversity); a `startpos`-only smoke check is still available via
`ALLOW_STARTPOS_GATE=1`, but it is explicitly *not* a strength measurement. `gate` also refuses to
call a low-diversity run PASS/FAIL — see `--min-diversity-ratio` below.

```bash
# One-shot regression (builds, runs against data/gate/openings_standard.sfen, prints PASS/FAIL/INCONCLUSIVE)
bash scripts/strength_regression.sh weights_new.bin weights_base.bin

# Or run the gate manually on an existing result JSON
cargo run --release -p sekirei-match-runner -- gate result.json \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10 --min-diversity-ratio 0.3
```

A full gate run can take long enough that babysitting it isn't practical.
`scripts/sprint_gate.sh` shards the opening suite by position across N short,
independently resumable sessions, then combines them into one gate-able result:

```bash
bash scripts/sprint_gate.sh weights_new.bin weights_base.bin 4       # 4 sprints, games-per-position=4
RUN_ID=my_run bash scripts/sprint_gate.sh weights_new.bin weights_base.bin 4  # resume my_run

# Sequential (SPRT) mode: checks after every sprint and stops as soon as
# it's decisive, instead of always running all N_SPRINTS.
SPRT=1 bash scripts/sprint_gate.sh weights_new.bin weights_base.bin 20
```

The match runner persists every game's outcome as a `<name>.jsonl` file alongside
`--json`'s `<name>.json`. `gate` reads that sibling file and re-runs the decision
through [veridict](https://github.com/kent-tokyo/veridict) (`--metric elo`), which
gates on the *confidence interval*, not the point estimate:

| Verdict | Condition |
|---------|-----------|
| **PASS** | CI lower bound ≥ pass threshold (default +20 elo) |
| **FAIL** | CI upper bound ≤ fail threshold (default −10 elo) |
| **INCONCLUSIVE** | CI straddles both — run more games |

`gate --sprt` runs a sequential test instead (H0: elo≤`elo0` vs H1: elo≥`elo1`,
default 0/20, alpha=beta=0.05 by default): it can reach a decisive PASS/FAIL well
before a fixed game count, since `alpha`/`beta` are the test's own guaranteed
error rates rather than a threshold on the point estimate — a PASS means "H1
accepted at that false-accept rate," not "proven ≥ `elo1`." `SPRT=1`'s
`MAX_GAMES` (default 1600) env var is a compute-budget cap, independent of
`N_SPRINTS`: a true effect strictly between `elo0` and `elo1` can otherwise
run indefinitely.

This is stricter than a plain point-estimate check: a lucky point estimate whose CI
still straddles zero is INCONCLUSIVE, not PASS. Elo/LOS point estimates (from the
same games) are still printed as a human-readable report line, per `elo_diff`,
`elo_ci_low`, `elo_ci_high`, `los` in the result JSON. Older result JSONs without a
`.jsonl` sibling (predating this change) fall back to the original point-estimate +
LOS check, noted explicitly in the gate's output.

Self-play Elo is only ever relative to whatever `engine2` was in that match — it has
no connection to an external rating pool (e.g. floodgate) on its own. If you have a
belief about that baseline's absolute rating, `--anchor <rating>` converts the gated
Elo effect into a rough estimate: `est_rating ≈ anchor + effect`. This is a directional
approximation, not a measurement — self-play Elo and population-pool Elo aren't the
same scale. There's no default; omit it and the output is unchanged.

```bash
cargo run --release -p sekirei-match-runner -- gate result.json --anchor 1850
# report: elo_diff=+82.6  los=96.9%  games=60
# veridict: metric=elo  effect=+82.6 elo  95% CI=[+41.0, +124.2]  CI lower bound ... meets the pass threshold ...
# PASS  est_rating≈1933 (anchor=1850)
```

## Benchmarks

Measured on Apple M4 Pro (`cargo build --release`, `target-cpu=native`).

| Metric | Value |
|--------|-------|
| Legal move generation (startpos) | ~5.5 µs / call |
| NNUE evaluate (startpos) | ~18.7 ns / call |
| Search depth 4 (startpos) | ~3.6 ms |
| Search NPS with NNUE (10 s time control) | ~1.1M nps, depth 13 |
| Test suite | 161 tests pass |

floodgate status: active testing; rating is currently volatile.

## Current Limitations

- NNUE weights are not bundled; train from floodgate CSA data or use the material eval fallback
- `setoption EvalFile` supported; in-game weight reload requires engine restart
- Pondering: supported (`go ponder` / `ponderhit`); enable via `setoption name Ponder value true`
- MultiPV: supported via `setoption name MultiPV value N`

## Name Origin

**SEKIREI** — *Shogi Engine for Kifu-Informed Reasoning and Efficient Inference*

Also セキレイ／鶺鴒, the Japanese word for wagtail — a small, nimble bird known for its quick,
darting movement. The name reflects the engine's focus on fast, speculative
search: committing to moves early, then correcting course as the tree develops.

## License

Licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you shall be dual licensed as above, without
any additional terms or conditions.

Sekirei is an original pure-Rust shogi engine. It does not include GPL-licensed
engine code. Ideas and algorithms may be studied from prior art, but this
project's implementation remains clean-room and compatible with its permissive
license.
