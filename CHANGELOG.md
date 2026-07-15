# Changelog

## Unreleased

## [0.3.0] – 2026-07-16

### Added
- `sekirei-train --lr <f> --lr-schedule constant|step-half|cosine --min-lr <f> --warmup-epochs <n>` — replaces the previously hardcoded `0.001 * 0.5^(epoch-1)` schedule (both `--games` and `--positions` paths). `step-half` remains the default and reproduces the old formula exactly. `min-lr` floors every schedule, not just `cosine` — unfloored `step-half` decays toward zero (~2e-9 by epoch 20), which made an early-stopped checkpoint hard to interpret (undertrained, or already past the point where the schedule mattered?).
- `sekirei-train --validation-ratio <f>` now also works on the `--games` (CSA) path, not just `--positions` — held-out split by **game index** (leakage-safe: every sampled position from a game lands on one side, never split across train/valid). Validation loss uses a new `Trainer::eval_game`, sharing the exact training objective (including the `--wdl-lambda` blend) via a shared `position_teacher` helper — routing through the positions path's `eval_positions` would have silently validated against a different (eval-only) objective whenever `--wdl-lambda` was set.
- Per-epoch training diagnostics (`diagnostics.rs`): FT/L2 "ever active"/"ever saturated" ratios (epoch-scoped — a dead neuron is one that never fires across a whole epoch, not a single-sample zero read), output mean/std, whole-parameter-vector update norm between epochs, post-quantization FT zero ratio. Printed per epoch and written to checkpoint `.meta.json`.
- Checkpoint `.meta.json` (both paths) now also records `git_commit`, `dataset_hash` (path+size fingerprint), `split_hash` (fingerprint of which positions/games landed in validation — distinguishes two different splits of the same dataset, which `dataset_hash` alone can't), `train_games`/`valid_games` (game-level counts, CSA path only), `architecture`, and the new LR-schedule fields. The `--games` (CSA) path writes `.meta.json` for the first time — previously only `--positions` did.
- CSA-path teacher-search caching: `position_teacher` (shared by `train_game`/`eval_game`) now caches each position's raw search score across epochs, mirroring the fix `eval_positions` already had on the positions path. Previously every epoch re-ran a full label-depth search on every sampled position — on a 20-epoch run this made epochs 2-20 pure repeat work (~2h/epoch observed, flat, zero speedup). `cache_hits`/`cache_misses` are logged per epoch and recorded in `.meta.json`.
- `sekirei-train --lr-schedule-epochs <n>` — shapes the LR curve for a horizon independent of `--epochs`, so a short run can reproduce the first N epochs of a longer schedule (e.g. `--epochs 3 --lr-schedule-epochs 20`). Defaults to `--epochs`, reproducing prior behavior exactly when omitted. Rejects `schedule_epochs=0`, `warmup_epochs > schedule_epochs`, and `schedule_epochs < epochs` outright instead of silently clamping (an earlier attempt at this reproduction always passed `--epochs` as the schedule horizon, compressing the whole cosine decay into the short run instead of reusing the long run's curve — this flag is the fix). `.meta.json` records both `epochs` and `lr_schedule_epochs`.
- Validation-set output stats (`ValidStats`, `.meta.json`, per-epoch log line): `valid_output_min`/`valid_output_max`/`valid_output_range`, computed directly (no variance-formula cancellation) alongside the existing `valid_output_std`. `std`'s cancellation can round a genuinely nonzero spread down to an exact `0.000` near total output collapse; `range` disambiguates "truly constant" (`range == 0.0`) from "collapsed but not literally frozen."
- `sekirei-match gate --sprt [--elo0 0] [--elo1 20] [--alpha 0.05] [--beta 0.05] [--sprt-variant wald|trinomial]` — sequential (SPRT) gate verdict alongside the existing CI-based one, using veridict's `sprt::run`. Reaches PASS/FAIL as soon as the log-likelihood ratio crosses a Wald boundary, often well before a fixed game count.
- `scripts/sprint_gate.sh SPRT=1` — opt-in early stopping: checks `gate --sprt` after every sprint and stops as soon as it's decisive. `MAX_GAMES` (default 1600) is a hard compute-budget cap, independent of `N_SPRINTS`, for the case where the true effect sits between `elo0`/`elo1` and SPRT would otherwise keep going indefinitely.
- `sekirei-train --wdl-lambda <f>` (`--games`/CSA path only) — blends the game's own result into the training teacher: `teacher = λ·eval_teacher + (1-λ)·wdl_target`. Positions from `GameResult::Unknown` games (aborted, timed out, illegal move, ...) fall back to eval-only, since there's no result signal to mix in for those.
- `csa.rs`: `GameResult` now recognizes `%SENNICHITE` (repetition → draw) and `%KACHI` (27-point declaration → win for the side that just moved) — previously both silently fell into `Unknown` (a combined ~13.9k games, ~3.9% of the current floodgate corpus).
- `scripts/cleanup_runs.sh` — prunes `data/runs/*/stage1`-`stage3` intermediates (raw extracts/observations/scored jsonl, often multi-GB) once a run has a `manifest.json` and is older than `MIN_AGE_DAYS` (default 3); skips runs referenced by name in `scripts/*.sh` (live cross-run dependencies) and runs with no manifest (still running or ad-hoc). Dry run by default, `APPLY=1` to delete. Wired into `redo_quietset_bc.sh`/`train_with_loss_mining.sh`/`train_with_shogiesa_quietset.sh` so old runs get pruned automatically each time a new one starts.
- `sekirei-train --grad-clip-norm <f>` (global) and independent per-layer `--ft-clip-norm`/`--l2-clip-norm`/`--out-clip-norm` — gradient-norm clipping, each layer scaled against its own gradient norm only when a per-layer threshold is set. All optional, default unset/disabled, no auto-enable. Investigated as a fix for output-layer scale runaway and an epoch-1 output collapse; **not adopted** — neither global nor output-only clipping improved either failure mode, and FT's gradient tail being measurably trimmed didn't translate into better generalization either way. Full writeup: `docs/experiments/global_gradient_clipping.md`.
- `sekirei-train --l2-bias-init <f>` — tunable L2 layer bias at initialization (default `0.5`, matching prior hardcoded behavior exactly when omitted). Investigated as a fix for the same epoch-1 collapse after warmup ruled out update magnitude as the cause; eliminates dead-at-init L2 neurons cleanly (0% in 3/3 seeds tested) but **not adopted** — real training's epoch-1 update relocates the collapse to the opposite ClippedReLU wall (saturation) rather than resolving it. Full writeup: `docs/experiments/l2_bias_init.md`.
- `docs/experiments/output_warmup.md` — `--warmup-epochs` tested against the same epoch-1 collapse; **not adopted** (dead-neuron count is completely insensitive to a 2× change in the first epoch's LR).
- `docs/training_lessons.md` and `docs/experiments/` — durable NNUE-training design notes and a running record of single-variable training experiments (promoted and rejected alike), linked from the README.
- `scripts/gate_dashboard.py`: embedded review panels (training-pipeline, individual gate result, project-wide trend) with deterministic Python-computed numbers/verdicts and an optional, strictly descriptive LLM narrative (never allowed to originate a number or override a verdict). Three distinct, deliberately non-interchangeable verdict vocabularies — gate: `PASS`/`FAIL`/`INCONCLUSIVE`; pipeline: `HEALTHY`/`WARNING`/`INSUFFICIENT_DATA`/`INVALID`; project trend: `IMPROVING`/`MIXED`/`FLAT`/`REGRESSING`/`INSUFFICIENT_EVIDENCE` with an explicit confidence level and positive/negative evidence lists, never a bare pass/fail — so a numerically healthy training run is never conflated with a playing-strength claim.

### Fixed
- **`TrainWeights::new()` → `new_seeded(seed)`: broke a symmetry-collapse bug present in every trained network to date.** `ft`/`l2`/`out` were zero-initialized; with no source of asymmetry, every unit within a layer receives an identical gradient every step (backprop through a uniform downstream weight is itself uniform), so the whole net converges to and stays at effective width 1 per layer forever, no matter how much data or how many epochs. Confirmed by parsing real trained weights (`v007` through `v012`): every FT row, every L2 row, and `out` were each a single repeated scalar, variance exactly 0.0 — the declared 256/32-wide architecture was training as a linear (KP-style) evaluator. Fix: seeded He/Kaiming-uniform init for `ft`/`l2`/`out` (biases unchanged — they solve a narrower, unrelated dead-ReLU problem). `Trainer::new(seed)` / `--seed` (already existed for validation split and source_cap) now also seeds weight init, so training stays fully reproducible for a fixed seed. Verified: post-init variance > 0 in every layer, stays > 0 after training, and two identical `--seed`-fixed runs (`--label-depth 1` and `--label-depth 4`, the latter exercising the rayon-parallel search path) produced byte-identical output weight files despite differing wall-clock schedules.

## [0.2.4] – 2026-06-28

### Added
- `sekirei-train --positions <jsonl>` — accept a [shogiesa](https://github.com/kent-tokyo/shogiesa) `positions.jsonl` file as an alternative to `--games`; skips CSA parsing and trains from pre-extracted SFENs with phase/side/source metadata.
- `PositionSample`: carries `phase`, `side_to_move`, `ply`, `source` from shogiesa tags for training control.
- `--phase-weights <spec>` — per-phase loss multipliers (e.g. `opening=0.5,middlegame=1.0,endgame=1.2`).
- `--side-balance` — equalise black/white sample weights based on training-split distribution.
- `--source-cap <N>` — deterministic hash-based per-source sample cap (seed-reproducible, order-independent).
- `--validation-ratio <f>` / `--seed <n>` — hold-out split via SFEN hash; logs `loss_raw` and `loss_weighted` per epoch.
- `--checkpoint-dir <dir>` — save epoch checkpoints to a custom directory with `.meta.json` (training params + sample counts).
- `--teacher-cache <path>` / `--reuse-teacher-cache` — cache teacher scores (sfen → score_cp) to JSONL; epoch 2+ skips search entirely on cache hits.

## [0.2.3] – 2026-06-28

### Added
- `sekirei-train --label-threshold-cp <n>` — configurable adv/equal/disadv boundary (default: 120 cp).
- Epoch stats log: `missing_rate`, `avg_weight`, `matched` counts printed per epoch when `--scored` is active; `missing_rate > 50%` triggers a SFEN-mismatch warning.
- `Trainer::reset_epoch_stats()` — resets `total_loss / total_count / total_weight / dropped_missing` between epochs so per-epoch stats are correct.

### Fixed
- `avg_loss` now divides by `total_weight` (sum of stability weights) instead of `total_count`; previously under-reported loss when `--stability-weighted` was active.
- `scored.rs`: duplicate SFENs in the scored JSONL are now averaged (previously last-wins, which made results dependent on file ordering); switched JSON parsing from manual string scan to `serde_json`.

## [0.2.2] – 2026-06-28

### Added
- `setoption MoveOverhead` (default 50 ms) — subtracts network latency from time budget.
- `setoption Ponder` option declaration; `go ponder` treated as infinite search.
- `ponderhit` command — aborts ponder search; GUI follows with a real `go`.
- `sekirei-train --export <path>` — exports observation JSONL for quietset stability filtering.
- `sekirei-train --depths <list>` — comma-separated search depths for export (default: `4,6,8`).
- `sekirei-match-runner --games-per-position <n>` — cover-all mode: play N games per SFEN entry.
- `sekirei-train --quiet`, `--min-ply`, `--label-depth` — quiet position filtering based on "Study of the Proper NNUE Dataset".
- `setoption Threads` — configure rayon global thread pool from GUI.

### Fixed
- **`go depth N` time cap**: pure depth search (no clock args) no longer capped at 50 ms.
- **TT size**: `Tt::new` now uses floor-power-of-two; previously halved capacity for power-of-2 inputs (e.g. 64 MB → 32 MB).
- **Root TT bound**: stores `Bound::Lower` on fail-high instead of always `Bound::Exact`.
- **USI search thread race**: `JoinHandle` now stored and joined on `stop`/`usinewgame`/`go`/`quit`; prevents stale `bestmove` output.
- Time control: tighter divisor (÷15) when < 30 s remain; panic mode when < 5 s and byoyomi exists.
- CSA client: `dotenvy` loads `.env`; env vars renamed `FLOODGATE_ACCOUNT` / `FLOODGATE_TRIP`.

## [0.2.0] – 2026-06-28

### Added
- Match runner: Elo rating, CI, LOS, illegal-move detection, repetition draw, SFEN openings.
- `SpeculativeSearcher` enabled in USI; king-capture panics fixed.
- NNUE training pipeline improvements.
- GitHub Actions CI + smoke job; all clippy warnings fixed.
- `setoption EvalFile` support in USI engine.
- CI pre-commit hook (`.githooks/pre-commit`).

### Fixed
- Mate score direction in `spec_alpha_beta`.
- NMP fail-soft + depth-scaled LMR formula.
- **CSA time tracking**: `parse_time_from_echo` now handles `+9796FU,T18` server echo format; `time_left_ms` was never decremented before.
- Read `Total_Time`/`Byoyomi`/`Increment` from `Game_Summary` header instead of parsing the game_id string.

## [0.1.0] – Initial

- NNUE-based shogi engine with alpha-beta search.
- CSA v2.2 TCP client for floodgate.
- USI protocol support.
