//! NNUE training loop — supervised learning from search-eval labels, with an
//! optional WDL (game-result) term for the CSA path.
#![allow(clippy::needless_range_loop)] // index-based loops match matrix layout; don't change
//!
//! # Architecture
//!   Input → FT (L1=256, per perspective) → ClippedReLU → L2 (32) → ClippedReLU → Out
//!
//! # Algorithm
//!   eval_teacher = clamp(search_score_cp, ±600)
//!   teacher      = eval_teacher                                  (wdl_lambda = None, default)
//!              or  λ·eval_teacher + (1-λ)·wdl_target              (wdl_lambda = Some(λ), CSA path only)
//!     where wdl_target = (game_result_from_stm_perspective − 0.5) × 1200,
//!     mapping loss/draw/win to ∓600/0/±600 on the same scale as eval_teacher.
//!     `GameResult::Unknown` games (see `csa.rs`) fall back to pure
//!     eval_teacher for that position -- there's no result signal to mix in.
//!   loss = (score − teacher)²   where score = output / 64.0
//!   gradients backpropagated through ClippedReLU layers
//!   weights updated with Adam
//!
//! Mixing via a single blended teacher, rather than a two-term loss
//! `λ(x−a)² + (1−λ)(x−b)²`, is deliberate, not a shortcut: the two have
//! identical gradients (`d/dx` of the two-term loss is `2(x − (λa+(1−λ)b))`,
//! exactly the blended-teacher squared-error gradient), so blending first
//! reuses `train_position`'s existing single-teacher backprop unchanged.
//!
//! FT weights are quantised to i16 at save time; L2/out stay f32.

use std::collections::HashMap;

use sekirei_core::{
    board::Board,
    color::Color,
    movegen::is_in_check,
    nnue::{INPUT, L1, L2, NnueWeights, feature_index, hand_feature_index},
    piece::PieceKind,
    search::{SearchConfig, Searcher},
    sfen::board_to_sfen,
    tt::Tt,
};

use crate::csa::{CsaGame, GameResult};

/// The sampled position's own game-result signal, on the same ±600
/// centipawn scale as a clamped eval teacher (loss=-600, draw=0, win=+600),
/// from `stm`'s perspective. `None` for `GameResult::Unknown` -- there is no
/// win/draw/loss signal to give for an aborted/timed-out/illegal-move game,
/// and guessing one (e.g. treating it as a draw) would add noise instead of
/// signal (see `csa.rs`'s `GameResult` doc).
fn wdl_target_cp(result: GameResult, stm: Color) -> Option<f32> {
    let wdl = match result {
        GameResult::BlackWin => {
            if stm == Color::Black {
                1.0
            } else {
                0.0
            }
        }
        GameResult::WhiteWin => {
            if stm == Color::White {
                1.0
            } else {
                0.0
            }
        }
        GameResult::Draw => 0.5,
        GameResult::Unknown => return None,
    };
    Some((wdl - 0.5) * 1200.0)
}

// ---- Learning-rate schedule ----

/// Per-epoch learning-rate schedule. `StepHalf` is today's original (and
/// default) behaviour, exposed as a named option instead of a hardcoded
/// formula so it can be compared against alternatives without editing
/// source -- the schedule itself became a suspect once a gated candidate
/// turned out to have been promoted from only 3 of 20 scheduled epochs,
/// at a point `StepHalf` had already decayed the LR to 1/4 of its start
/// (see `tasks/lessons.md`, 2026-07-13 Gate B entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LrSchedule {
    /// Fixed at `base_lr` (after warmup) for the whole run.
    Constant,
    /// `base_lr * 0.5^(epoch-1)` -- halves every epoch. Matches every
    /// training run before this flag existed.
    StepHalf,
    /// Cosine decay from `base_lr` down to `min_lr`, reaching `min_lr`
    /// exactly at the final epoch.
    Cosine,
}

impl LrSchedule {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "constant" => Some(LrSchedule::Constant),
            "step-half" => Some(LrSchedule::StepHalf),
            "cosine" => Some(LrSchedule::Cosine),
            _ => None,
        }
    }
}

/// Computes the learning rate for `epoch` (1-indexed) out of `total_epochs`.
/// The first `warmup_epochs` epochs ramp linearly from 0 to `base_lr`
/// (epoch `warmup_epochs` itself lands exactly on `base_lr`); the chosen
/// `schedule` then governs decay over the remaining epochs. `min_lr` is a
/// floor applied to every schedule, not just `Cosine` -- without it,
/// `StepHalf` decays toward zero forever on a long run (by epoch 20 it's
/// already ~2e-9), which is itself part of what made an early-stopped
/// `StepHalf` checkpoint hard to interpret: was the recipe undertrained,
/// or had the schedule already made epoch 4+ pointless?
pub fn compute_lr(
    schedule: LrSchedule,
    base_lr: f32,
    min_lr: f32,
    epoch: u32,
    total_epochs: u32,
    warmup_epochs: u32,
) -> f32 {
    if warmup_epochs > 0 && epoch <= warmup_epochs {
        return (base_lr * epoch as f32 / warmup_epochs as f32).max(min_lr);
    }
    let e = epoch.saturating_sub(warmup_epochs).max(1);
    let post_total = total_epochs.saturating_sub(warmup_epochs).max(1);
    let lr = match schedule {
        LrSchedule::Constant => base_lr,
        LrSchedule::StepHalf => base_lr * 0.5_f32.powi((e - 1) as i32),
        LrSchedule::Cosine => {
            // Denominator is (post_total - 1), not post_total, so the last
            // epoch's progress is exactly 1.0 -- cos(pi) = -1 -> lr = min_lr
            // precisely on the final epoch, not asymptotically close to it.
            let denom = post_total.saturating_sub(1).max(1) as f32;
            let progress = ((e - 1) as f32 / denom).min(1.0);
            min_lr + 0.5 * (base_lr - min_lr) * (1.0 + (std::f32::consts::PI * progress).cos())
        }
    };
    lr.max(min_lr)
}

// ---- Deterministic PRNG for weight init (same LCG constants as
// sekirei-match-runner's `Lcg` -- Knuth MMIX) ----

struct Lcg(u64);
impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    /// Uniform f32 in [-bound, bound].
    fn uniform(&mut self, bound: f32) -> f32 {
        let u = self.next_u64() as f64 / u64::MAX as f64; // [0, 1]
        ((u * 2.0 - 1.0) as f32) * bound
    }
}

/// Kaiming/He uniform bound for a layer with `fan_in` inputs feeding a
/// ClippedReLU, per PyTorch's default `nn.Linear` init.
fn he_bound(fan_in: usize) -> f32 {
    (6.0 / fan_in as f32).sqrt()
}

// ---- Training weight container ----

pub struct TrainWeights {
    ft: Vec<f32>,      // INPUT × L1  (row-major: index = feat*L1 + neuron)
    ft_bias: Vec<f32>, // L1
    l2: Vec<f32>,      // 2*L1 × L2  (row-major: index = input_j*L2 + output_o)
    l2_bias: Vec<f32>, // L2
    out: Vec<f32>,     // L2
    out_bias: f32,

    // Adam first/second moments
    ft_m: Vec<f32>,
    ft_v: Vec<f32>,
    bias_m: Vec<f32>,
    bias_v: Vec<f32>,
    l2_m: Vec<f32>,
    l2_v: Vec<f32>,
    l2bias_m: Vec<f32>,
    l2bias_v: Vec<f32>,
    out_m: Vec<f32>,
    out_v: Vec<f32>,
    obias_m: f32,
    obias_v: f32,

    step: u64,
}

impl TrainWeights {
    /// Seeded He/Kaiming-uniform init. Zero-initialising `ft`/`l2`/`out`
    /// (the pre-2026-07-09 behaviour) never breaks symmetry: every unit in a
    /// layer starts identical and receives an identical gradient every step
    /// (backprop through a uniform downstream weight is itself uniform), so
    /// the whole net collapses to and stays at effective width 1 per layer
    /// forever -- confirmed by parsing real trained weights (`v007`..`v012`):
    /// every FT row, every L2 row, and `out` were each a single repeated
    /// scalar, variance exactly 0.0. The non-zero biases below predate this
    /// fix and only solved a narrower problem (all-zero forever, not
    /// symmetric-but-nonzero); kept as-is since they're still harmless.
    pub fn new_seeded(seed: u64) -> Self {
        let ft_len = INPUT * L1;
        let l2_len = 2 * L1 * L2;
        let out_len = L2;
        let mut rng = Lcg(seed ^ 0x9E37_79B9_7F4A_7C15);
        let ft_bound = he_bound(INPUT);
        let l2_bound = he_bound(2 * L1);
        let out_bound = he_bound(L2);
        TrainWeights {
            ft: (0..ft_len).map(|_| rng.uniform(ft_bound)).collect(),
            // Non-zero bias ensures ClippedReLU inputs are > 0 so gradients flow
            ft_bias: vec![0.5; L1],
            l2: (0..l2_len).map(|_| rng.uniform(l2_bound)).collect(),
            // Same reason as ft_bias: with l2 zero-initialized, a zero l2_bias makes
            // l2_acc land exactly on the ClippedReLU dead zone (== 0.0, gate is `> 0.0`),
            // permanently blocking gradient flow to l2/ft.
            l2_bias: vec![0.5; L2],
            out: (0..out_len).map(|_| rng.uniform(out_bound)).collect(),
            out_bias: 0.0,

            ft_m: vec![0.0; ft_len],
            ft_v: vec![0.0; ft_len],
            bias_m: vec![0.0; L1],
            bias_v: vec![0.0; L1],
            l2_m: vec![0.0; l2_len],
            l2_v: vec![0.0; l2_len],
            l2bias_m: vec![0.0; L2],
            l2bias_v: vec![0.0; L2],
            out_m: vec![0.0; out_len],
            out_v: vec![0.0; out_len],
            obias_m: 0.0,
            obias_v: 0.0,
            step: 0,
        }
    }

    /// Quantise FT to i16; L2/out stay f32.  Returns an NnueWeights ready for inference.
    pub fn to_nnue_weights(&self) -> NnueWeights {
        // FT: f32 → i16, scaled by FT_SCALE so small weights (≈±0.1) survive quantisation.
        // Inference must divide by FT_SCALE after ClippedReLU to recover the float equivalent.
        const FT_SCALE: f32 = 64.0;
        let mut ft = vec![[0i16; L1]; INPUT];
        for i in 0..INPUT {
            for j in 0..L1 {
                ft[i][j] = (self.ft[i * L1 + j] * FT_SCALE).clamp(-32767.0, 32767.0) as i16;
            }
        }
        let mut ft_bias = [0i16; L1];
        for (i, &v) in self.ft_bias.iter().enumerate() {
            ft_bias[i] = (v * FT_SCALE).clamp(-32767.0, 32767.0) as i16;
        }

        // L2 / out: f32 → f32 (no quantisation)
        let mut l2 = vec![[0.0f32; L2]; 2 * L1];
        for i in 0..2 * L1 {
            for o in 0..L2 {
                l2[i][o] = self.l2[i * L2 + o];
            }
        }
        let mut l2_bias = [0.0f32; L2];
        l2_bias.copy_from_slice(&self.l2_bias);

        let mut out = [0.0f32; L2];
        out.copy_from_slice(&self.out);

        NnueWeights {
            ft,
            ft_bias,
            l2,
            l2_bias,
            out,
            out_bias: self.out_bias,
        }
    }

    /// Flattened concat of every trainable parameter (not the Adam
    /// moments) -- used to compute a whole-network update-norm between
    /// two epoch boundaries, not for saving/loading.
    pub fn snapshot_params(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(
            self.ft.len()
                + self.ft_bias.len()
                + self.l2.len()
                + self.l2_bias.len()
                + self.out.len()
                + 1,
        );
        v.extend_from_slice(&self.ft);
        v.extend_from_slice(&self.ft_bias);
        v.extend_from_slice(&self.l2);
        v.extend_from_slice(&self.l2_bias);
        v.extend_from_slice(&self.out);
        v.push(self.out_bias);
        v
    }
}

// ---- Trainer ----

pub struct Trainer {
    pub weights: TrainWeights,
    pub total_loss: f64,
    pub total_count: u64,
    pub total_weight: f64,    // sum of weights (for avg_weight log)
    pub dropped_missing: u64, // positions skipped (not in scored map)
    pub lr: f32,
    // Per-epoch diagnostics. These are "ever" flags over the whole epoch,
    // not a per-sample snapshot -- a dead neuron is one that never fires
    // across an entire epoch of real data, not one that happens to read
    // zero on a single sample (that's what actually distinguishes the
    // 2026-07-09 capacity-collapse bug from normal ReLU sparsity). Only
    // `train_position`'s forward pass updates these; `eval_positions`/
    // `eval_game`'s validation-only forward passes must not, since these
    // measure what training actually touched, not what validation looked
    // at. Reset every epoch by `reset_epoch_stats`.
    pub ft_ever_active: Vec<bool>,
    pub ft_ever_saturated: Vec<bool>,
    pub l2_ever_active: Vec<bool>,
    pub l2_ever_saturated: Vec<bool>,
    pub output_sum: f64,
    pub output_sum_sq: f64,
    // CSA-path teacher-search cache counters (see `position_teacher`).
    // Reset every epoch by `reset_epoch_stats`, same as the diagnostics
    // above.
    pub cache_hits: u64,
    pub cache_misses: u64,
    searcher: Searcher,
}

impl Trainer {
    pub fn new(seed: u64) -> Self {
        let tt = Tt::new(4); // Tt::new returns Arc<Tt>
        Trainer {
            weights: TrainWeights::new_seeded(seed),
            total_loss: 0.0,
            total_count: 0,
            total_weight: 0.0,
            dropped_missing: 0,
            lr: 0.001,
            ft_ever_active: vec![false; L1],
            ft_ever_saturated: vec![false; L1],
            l2_ever_active: vec![false; L2],
            l2_ever_saturated: vec![false; L2],
            output_sum: 0.0,
            output_sum_sq: 0.0,
            cache_hits: 0,
            cache_misses: 0,
            searcher: Searcher::new(tt),
        }
    }

    /// Train on a slice of PositionSamples (from shogiesa positions.jsonl).
    /// `teacher_cache`: sfen → score_cp; cache hits skip search entirely.
    /// `new_entries`: receives (sfen, score_cp) for each search actually run (cache miss).
    #[allow(clippy::too_many_arguments)]
    pub fn train_positions(
        &mut self,
        samples: &[crate::positions::PositionSample],
        label_depth: u32,
        scored: &HashMap<String, f32>,
        stability_weighted: bool,
        phase_weights: &HashMap<String, f32>,
        side_weights: &HashMap<String, f32>,
        teacher_cache: &HashMap<String, i32>,
        new_entries: &mut Vec<(String, i32)>,
    ) {
        for sample in samples {
            let sfen = sekirei_core::sfen::board_to_sfen(&sample.board);
            let stability = if scored.is_empty() {
                1.0f32
            } else {
                match scored.get(&sfen) {
                    Some(&s) => {
                        if stability_weighted {
                            s
                        } else {
                            1.0
                        }
                    }
                    None => {
                        self.dropped_missing += 1;
                        continue;
                    }
                }
            };
            let phase_w = phase_weights.get(&sample.phase).copied().unwrap_or(1.0);
            let side_w = side_weights
                .get(&sample.side_to_move)
                .copied()
                .unwrap_or(1.0);
            let weight = stability * phase_w * side_w;

            let score_cp = if let Some(&cp) = teacher_cache.get(&sfen) {
                cp
            } else {
                let config = SearchConfig {
                    max_depth: label_depth,
                    time_limit: None,
                    soft_limit: None,
                    multi_pv: 1,
                };
                let mut b = sample.board.clone();
                let cp = self.searcher.search(&mut b, config).score;
                new_entries.push((sfen, cp));
                cp
            };
            let teacher = (score_cp as f32).clamp(-600.0, 600.0);
            self.train_position(&sample.board, teacher, weight);
        }
    }

    /// Forward-only pass for validation loss (no weight updates).
    /// Returns `(loss_raw, loss_weighted, count)`.
    /// `loss_raw` = plain MSE; `loss_weighted` = MSE weighted by phase/side multipliers.
    /// Teacher scores are looked up in `teacher_cache` first, same as
    /// `train_positions` — without this, validation re-ran a real
    /// label-depth search on every sample on every epoch, even when the
    /// cache already had every score (this was the actual cause of a
    /// training run taking ~15 min/epoch on a fully-cached 10k dataset).
    pub fn eval_positions(
        &mut self,
        samples: &[crate::positions::PositionSample],
        label_depth: u32,
        phase_weights: &HashMap<String, f32>,
        side_weights: &HashMap<String, f32>,
        teacher_cache: &HashMap<String, i32>,
        new_entries: &mut Vec<(String, i32)>,
    ) -> (f64, f64, u64) {
        let mut loss_raw = 0.0f64;
        let mut loss_weighted = 0.0f64;
        let mut total_w = 0.0f64;
        let mut count = 0u64;
        for sample in samples {
            let sfen = sekirei_core::sfen::board_to_sfen(&sample.board);
            let teacher_cp = if let Some(&cp) = teacher_cache.get(&sfen) {
                cp
            } else {
                let config = SearchConfig {
                    max_depth: label_depth,
                    time_limit: None,
                    soft_limit: None,
                    multi_pv: 1,
                };
                let mut b = sample.board.clone();
                let cp = self.searcher.search(&mut b, config).score;
                new_entries.push((sfen, cp));
                cp
            };
            let teacher = (teacher_cp as f32).clamp(-600.0, 600.0);
            let score = self.forward(&sample.board);
            let err2 = ((score - teacher) * (score - teacher)) as f64;
            loss_raw += err2;
            let w = phase_weights.get(&sample.phase).copied().unwrap_or(1.0)
                * side_weights
                    .get(&sample.side_to_move)
                    .copied()
                    .unwrap_or(1.0);
            loss_weighted += w as f64 * err2;
            total_w += w as f64;
            count += 1;
        }
        let raw = if count > 0 {
            loss_raw / count as f64
        } else {
            0.0
        };
        let weighted = if total_w > 0.0 {
            loss_weighted / total_w
        } else {
            0.0
        };
        (raw, weighted, count)
    }

    /// Computes the teacher target for a single position: a clamped
    /// search eval, optionally blended with the game's own WDL result.
    /// Shared by `train_game` (updates weights) and `eval_game`
    /// (validation-only) so both measure against the exact same
    /// objective -- routing validation through a pure-eval-only path
    /// (like `eval_positions`) would silently validate against a
    /// different target than the one being trained whenever `wdl_lambda`
    /// is set, since `eval_positions` never blends in a WDL term.
    ///
    /// `cache` maps sfen -> raw search score (pre-clamp, pre-WDL-blend),
    /// mirroring `train_positions`/`eval_positions`'s `teacher_cache`. Only
    /// `eval_teacher` is cached, not the blended result: the same position
    /// can recur in different games with different results, so the WDL
    /// term is always recomputed from this call's own `result`/
    /// side-to-move. Without this, every epoch re-ran a real label-depth
    /// search on every sampled position -- the exact bug `eval_positions`'s
    /// doc comment describes already being fixed once on the positions path.
    #[allow(clippy::too_many_arguments)]
    fn position_teacher(
        &mut self,
        board: &mut Board,
        result: GameResult,
        label_depth: u32,
        wdl_lambda: Option<f32>,
        cache: &mut HashMap<String, i32>,
    ) -> f32 {
        let sfen = board_to_sfen(board);
        let score_cp = if let Some(&cp) = cache.get(&sfen) {
            self.cache_hits += 1;
            cp
        } else {
            self.cache_misses += 1;
            let config = SearchConfig {
                max_depth: label_depth,
                time_limit: None,
                soft_limit: None,
                multi_pv: 1,
            };
            let cp = self.searcher.search(board, config).score;
            cache.insert(sfen, cp);
            cp
        };
        let eval_teacher = (score_cp as f32).clamp(-600.0, 600.0);
        match (wdl_lambda, wdl_target_cp(result, board.side_to_move)) {
            (Some(lambda), Some(wdl_target)) => lambda * eval_teacher + (1.0 - lambda) * wdl_target,
            _ => eval_teacher,
        }
    }

    /// Train on a single game.  Samples every `sample_every` plies.
    /// `wdl_lambda`: `None` trains on `eval_teacher` alone (default,
    /// backward-compatible). `Some(λ)` blends in the game's own result from
    /// each sampled position's side-to-move perspective, skipping the blend
    /// (falling back to pure eval) for `GameResult::Unknown` games, since
    /// there's no result signal to mix in for those (see `csa.rs`).
    #[allow(clippy::too_many_arguments)]
    pub fn train_game(
        &mut self,
        game: &CsaGame,
        sample_every: usize,
        quiet: bool,
        min_ply: usize,
        label_depth: u32,
        scored: &HashMap<String, f32>,
        stability_weighted: bool,
        wdl_lambda: Option<f32>,
        cache: &mut HashMap<String, i32>,
    ) {
        let mut board = Board::startpos();

        for (ply, &mv) in game.moves.iter().enumerate() {
            if ply < min_ply || ply % sample_every != 0 {
                board.do_move(mv);
                continue;
            }

            if quiet {
                // skip positions in check (tactically unstable)
                if is_in_check(&board, board.side_to_move) {
                    board.do_move(mv);
                    continue;
                }
                // skip if next move is a capture (tactically unstable)
                if board.piece_at(mv.to).is_some() {
                    board.do_move(mv);
                    continue;
                }
            }

            // quietset filter / weighting
            let weight = if scored.is_empty() {
                1.0f32
            } else {
                let sfen = board_to_sfen(&board);
                match scored.get(&sfen) {
                    Some(&s) => {
                        if stability_weighted {
                            s
                        } else {
                            1.0
                        }
                    }
                    None => {
                        self.dropped_missing += 1;
                        board.do_move(mv);
                        continue; // not in keep set
                    }
                }
            };

            let teacher =
                self.position_teacher(&mut board, game.result, label_depth, wdl_lambda, cache);
            self.train_position(&board, teacher, weight);

            board.do_move(mv);
        }
    }

    /// Forward-only pass over a single game for validation loss (no
    /// weight updates, no epoch-stat/diagnostic-counter mutation --
    /// validation measures what training touched, not what validation
    /// itself looked at). Mirrors `train_game`'s replay/sample loop, but
    /// returns a plain `(loss_sum, count)`: CSA-path training has no
    /// weighted-loss axis (unlike the positions path's phase/side
    /// weights) to validate against, so there's nothing to weight here.
    #[allow(clippy::too_many_arguments)]
    pub fn eval_game(
        &mut self,
        game: &CsaGame,
        sample_every: usize,
        quiet: bool,
        min_ply: usize,
        label_depth: u32,
        wdl_lambda: Option<f32>,
        cache: &mut HashMap<String, i32>,
    ) -> (f64, u64) {
        let mut board = Board::startpos();
        let mut loss_sum = 0.0f64;
        let mut count = 0u64;

        for (ply, &mv) in game.moves.iter().enumerate() {
            if ply < min_ply || ply % sample_every != 0 {
                board.do_move(mv);
                continue;
            }
            if quiet {
                if is_in_check(&board, board.side_to_move) {
                    board.do_move(mv);
                    continue;
                }
                if board.piece_at(mv.to).is_some() {
                    board.do_move(mv);
                    continue;
                }
            }

            let teacher =
                self.position_teacher(&mut board, game.result, label_depth, wdl_lambda, cache);
            let score = self.forward(&board);
            let err = (score - teacher) as f64;
            loss_sum += err * err;
            count += 1;

            board.do_move(mv);
        }

        (loss_sum, count)
    }

    /// Forward pass only — returns score without any weight update.
    fn forward(&self, board: &Board) -> f32 {
        let stm = board.side_to_move;
        let w = &self.weights;
        let mut acc_us = w.ft_bias.clone();
        let mut acc_them = acc_us.clone();
        for feat in &active_features(board, stm) {
            let base = feat * L1;
            for j in 0..L1 {
                acc_us[j] += w.ft[base + j];
            }
        }
        for feat in &active_features(board, stm.flip()) {
            let base = feat * L1;
            for j in 0..L1 {
                acc_them[j] += w.ft[base + j];
            }
        }
        let relu_us: Vec<f32> = acc_us.iter().map(|&x| x.clamp(0.0, 127.0)).collect();
        let relu_them: Vec<f32> = acc_them.iter().map(|&x| x.clamp(0.0, 127.0)).collect();
        let mut l2_acc = w.l2_bias.clone();
        for j in 0..L1 {
            let base_us = j * L2;
            let base_them = (L1 + j) * L2;
            for o in 0..L2 {
                l2_acc[o] += relu_us[j] * w.l2[base_us + o];
                l2_acc[o] += relu_them[j] * w.l2[base_them + o];
            }
        }
        let relu_l2: Vec<f32> = l2_acc.iter().map(|&x| x.clamp(0.0, 127.0)).collect();
        let mut output = w.out_bias;
        for o in 0..L2 {
            output += relu_l2[o] * w.out[o];
        }
        output / 64.0
    }

    /// One SGD step on a single position. `weight` scales the loss (quietset stability).
    fn train_position(&mut self, board: &Board, teacher: f32, weight: f32) {
        let stm = board.side_to_move;
        let w = &self.weights;

        // ── Forward pass ──────────────────────────────────────────────────────

        // FT accumulation
        let mut acc_us = w.ft_bias.clone();
        let mut acc_them = acc_us.clone();

        let active_us = active_features(board, stm);
        let active_them = active_features(board, stm.flip());

        for feat in &active_us {
            let base = feat * L1;
            for j in 0..L1 {
                acc_us[j] += w.ft[base + j];
            }
        }
        for feat in &active_them {
            let base = feat * L1;
            for j in 0..L1 {
                acc_them[j] += w.ft[base + j];
            }
        }

        // FT ClippedReLU [0, 127]
        let relu_us: Vec<f32> = acc_us.iter().map(|&x| x.clamp(0.0, 127.0)).collect();
        let relu_them: Vec<f32> = acc_them.iter().map(|&x| x.clamp(0.0, 127.0)).collect();
        for j in 0..L1 {
            if relu_us[j] > 0.0 || relu_them[j] > 0.0 {
                self.ft_ever_active[j] = true;
            }
            if relu_us[j] >= 127.0 || relu_them[j] >= 127.0 {
                self.ft_ever_saturated[j] = true;
            }
        }

        // L2 accumulation
        let mut l2_acc = w.l2_bias.clone(); // Vec<f32> len=L2
        for j in 0..L1 {
            let a = relu_us[j];
            let b = relu_them[j];
            let base_us = j * L2;
            let base_them = (L1 + j) * L2;
            for o in 0..L2 {
                l2_acc[o] += a * w.l2[base_us + o];
                l2_acc[o] += b * w.l2[base_them + o];
            }
        }

        // L2 ClippedReLU [0, 127]
        let relu_l2: Vec<f32> = l2_acc.iter().map(|&x| x.clamp(0.0, 127.0)).collect();
        for o in 0..L2 {
            if relu_l2[o] > 0.0 {
                self.l2_ever_active[o] = true;
            }
            if relu_l2[o] >= 127.0 {
                self.l2_ever_saturated[o] = true;
            }
        }

        // Output
        let mut output = w.out_bias;
        for o in 0..L2 {
            output += relu_l2[o] * w.out[o];
        }
        let score = output / 64.0;
        self.output_sum += score as f64;
        self.output_sum_sq += (score as f64) * (score as f64);

        // ── Loss ──────────────────────────────────────────────────────────────

        let err = score - teacher;
        self.total_loss += (weight as f64) * (err * err) as f64;
        self.total_count += 1;
        self.total_weight += weight as f64;

        // ── Backward pass ─────────────────────────────────────────────────────

        let d_score = weight * 2.0 * err;
        let d_output = d_score / 64.0;

        // Output layer gradients
        let mut d_out = vec![0.0f32; L2];
        for o in 0..L2 {
            d_out[o] = d_output * relu_l2[o];
        }
        let d_out_bias = d_output;

        // Backprop through L2 ClippedReLU
        let mut d_l2_acc = [0.0f32; L2];
        for o in 0..L2 {
            if l2_acc[o] > 0.0 && l2_acc[o] < 127.0 {
                d_l2_acc[o] = d_output * self.weights.out[o];
            }
        }

        // L2 weight gradients and propagate to FT
        let mut d_l2 = vec![0.0f32; 2 * L1 * L2];
        let mut d_l2_bias = vec![0.0f32; L2];
        let mut d_relu_us = vec![0.0f32; L1];
        let mut d_relu_them = vec![0.0f32; L1];

        for j in 0..L1 {
            let base_us = j * L2;
            let base_them = (L1 + j) * L2;
            for o in 0..L2 {
                let g = d_l2_acc[o];
                d_l2[base_us + o] += g * relu_us[j];
                d_l2[base_them + o] += g * relu_them[j];
                d_relu_us[j] += g * self.weights.l2[base_us + o];
                d_relu_them[j] += g * self.weights.l2[base_them + o];
            }
        }
        d_l2_bias[..L2].copy_from_slice(&d_l2_acc[..L2]);

        // Backprop through FT ClippedReLU
        let mut d_acc_us = vec![0.0f32; L1];
        let mut d_acc_them = vec![0.0f32; L1];
        for j in 0..L1 {
            if acc_us[j] > 0.0 && acc_us[j] < 127.0 {
                d_acc_us[j] = d_relu_us[j];
            }
            if acc_them[j] > 0.0 && acc_them[j] < 127.0 {
                d_acc_them[j] = d_relu_them[j];
            }
        }

        // FT weight gradients (sparse)
        let mut d_ft = vec![0.0f32; INPUT * L1];
        let mut d_bias = vec![0.0f32; L1];

        for feat in &active_us {
            let base = feat * L1;
            for j in 0..L1 {
                d_ft[base + j] += d_acc_us[j];
            }
        }
        for feat in &active_them {
            let base = feat * L1;
            for j in 0..L1 {
                d_ft[base + j] += d_acc_them[j];
            }
        }
        for j in 0..L1 {
            d_bias[j] = d_acc_us[j] + d_acc_them[j];
        }

        // ── Adam update ───────────────────────────────────────────────────────

        self.weights.step += 1;
        let t = self.weights.step;
        let lr = self.lr;

        adam_update_slice(
            &mut self.weights.ft,
            &mut self.weights.ft_m,
            &mut self.weights.ft_v,
            &d_ft,
            lr,
            t,
        );
        adam_update_slice(
            &mut self.weights.ft_bias,
            &mut self.weights.bias_m,
            &mut self.weights.bias_v,
            &d_bias,
            lr,
            t,
        );
        adam_update_slice(
            &mut self.weights.l2,
            &mut self.weights.l2_m,
            &mut self.weights.l2_v,
            &d_l2,
            lr,
            t,
        );
        adam_update_slice(
            &mut self.weights.l2_bias,
            &mut self.weights.l2bias_m,
            &mut self.weights.l2bias_v,
            &d_l2_bias,
            lr,
            t,
        );
        adam_update_slice(
            &mut self.weights.out,
            &mut self.weights.out_m,
            &mut self.weights.out_v,
            &d_out,
            lr,
            t,
        );
        adam_update_scalar(
            &mut self.weights.out_bias,
            &mut self.weights.obias_m,
            &mut self.weights.obias_v,
            d_out_bias,
            lr,
            t,
        );
    }

    pub fn avg_loss(&self) -> f64 {
        if self.total_weight > 0.0 {
            self.total_loss / self.total_weight
        } else {
            0.0
        }
    }

    pub fn reset_epoch_stats(&mut self) {
        self.total_loss = 0.0;
        self.total_count = 0;
        self.total_weight = 0.0;
        self.dropped_missing = 0;
        self.ft_ever_active.iter_mut().for_each(|b| *b = false);
        self.ft_ever_saturated.iter_mut().for_each(|b| *b = false);
        self.l2_ever_active.iter_mut().for_each(|b| *b = false);
        self.l2_ever_saturated.iter_mut().for_each(|b| *b = false);
        self.output_sum = 0.0;
        self.output_sum_sq = 0.0;
        self.cache_hits = 0;
        self.cache_misses = 0;
    }
}

// ---- Active feature extraction ----

fn active_features(board: &Board, perspective: Color) -> Vec<usize> {
    const ALL_KINDS: [PieceKind; 14] = [
        PieceKind::Fu,
        PieceKind::Kyou,
        PieceKind::Kei,
        PieceKind::Gin,
        PieceKind::Kin,
        PieceKind::Kaku,
        PieceKind::Hisha,
        PieceKind::Ou,
        PieceKind::Tokin,
        PieceKind::Narikyo,
        PieceKind::Narikei,
        PieceKind::Narigin,
        PieceKind::Uma,
        PieceKind::Ryu,
    ];
    const HAND_KINDS: [PieceKind; 7] = [
        PieceKind::Fu,
        PieceKind::Kyou,
        PieceKind::Kei,
        PieceKind::Gin,
        PieceKind::Kin,
        PieceKind::Kaku,
        PieceKind::Hisha,
    ];

    let mut features = Vec::with_capacity(60);
    // Board features
    for &kind in &ALL_KINDS {
        for color in [Color::Black, Color::White] {
            let mut bb = board.pieces(color, kind);
            while let Some(sq) = bb.pop_lsb() {
                features.push(feature_index(sq, kind, color, perspective));
            }
        }
    }
    // Hand features: "≥ N pieces of kind K in hand" threshold features
    for &kind in &HAND_KINDS {
        for color in [Color::Black, Color::White] {
            let count = board.hand(color).get(kind);
            for n in 1..=count {
                features.push(hand_feature_index(kind, n, color, perspective));
            }
        }
    }
    features
}

// ---- Adam helpers ----

fn adam_update_slice(
    params: &mut [f32],
    m: &mut [f32],
    v: &mut [f32],
    grads: &[f32],
    lr: f32,
    t: u64,
) {
    for i in 0..params.len() {
        adam_update_scalar(&mut params[i], &mut m[i], &mut v[i], grads[i], lr, t);
    }
}

#[inline]
fn adam_update_scalar(param: &mut f32, m: &mut f32, v: &mut f32, grad: f32, lr: f32, t: u64) {
    const B1: f32 = 0.9;
    const B2: f32 = 0.999;
    const EPS: f32 = 1e-8;

    *m = B1 * *m + (1.0 - B1) * grad;
    *v = B2 * *v + (1.0 - B2) * grad * grad;

    let m_hat = *m / (1.0 - B1.powi(t as i32));
    let v_hat = *v / (1.0 - B2.powi(t as i32));

    *param -= lr * m_hat / (v_hat.sqrt() + EPS);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variance(xs: &[f32]) -> f32 {
        let mean = xs.iter().sum::<f32>() / xs.len() as f32;
        xs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / xs.len() as f32
    }

    #[test]
    fn seeded_init_breaks_symmetry_within_each_layer() {
        let w = TrainWeights::new_seeded(42);
        // Any single FT row (one input feature's L1 contributions) must not
        // collapse to a single repeated scalar -- that's the exact failure
        // this init replaces (see `new_seeded`'s doc comment).
        assert!(variance(&w.ft[0..L1]) > 0.0);
        assert!(variance(&w.l2[0..L2]) > 0.0);
        assert!(variance(&w.out) > 0.0);
    }

    #[test]
    fn seeded_init_is_deterministic() {
        let a = TrainWeights::new_seeded(42);
        let b = TrainWeights::new_seeded(42);
        assert_eq!(a.ft, b.ft);
        assert_eq!(a.l2, b.l2);
        assert_eq!(a.out, b.out);
    }

    #[test]
    fn seeded_init_differs_across_seeds() {
        let a = TrainWeights::new_seeded(1);
        let b = TrainWeights::new_seeded(2);
        assert_ne!(a.ft, b.ft);
    }

    #[test]
    fn wdl_target_black_win_from_black_perspective_is_max() {
        assert_eq!(
            wdl_target_cp(GameResult::BlackWin, Color::Black),
            Some(600.0)
        );
    }

    #[test]
    fn wdl_target_black_win_from_white_perspective_is_min() {
        assert_eq!(
            wdl_target_cp(GameResult::BlackWin, Color::White),
            Some(-600.0)
        );
    }

    #[test]
    fn wdl_target_white_win_from_white_perspective_is_max() {
        assert_eq!(
            wdl_target_cp(GameResult::WhiteWin, Color::White),
            Some(600.0)
        );
    }

    #[test]
    fn wdl_target_white_win_from_black_perspective_is_min() {
        assert_eq!(
            wdl_target_cp(GameResult::WhiteWin, Color::Black),
            Some(-600.0)
        );
    }

    #[test]
    fn wdl_target_draw_is_zero_regardless_of_perspective() {
        assert_eq!(wdl_target_cp(GameResult::Draw, Color::Black), Some(0.0));
        assert_eq!(wdl_target_cp(GameResult::Draw, Color::White), Some(0.0));
    }

    #[test]
    fn wdl_target_unknown_result_has_no_signal() {
        assert_eq!(wdl_target_cp(GameResult::Unknown, Color::Black), None);
        assert_eq!(wdl_target_cp(GameResult::Unknown, Color::White), None);
    }

    #[test]
    fn compute_lr_step_half_matches_original_hardcoded_formula() {
        // No warmup, no min_lr floor -- must reproduce the exact pre-flag behaviour.
        assert_eq!(
            compute_lr(LrSchedule::StepHalf, 0.001, 0.0, 1, 20, 0),
            0.001
        );
        assert_eq!(
            compute_lr(LrSchedule::StepHalf, 0.001, 0.0, 2, 20, 0),
            0.0005
        );
        assert!((compute_lr(LrSchedule::StepHalf, 0.001, 0.0, 3, 20, 0) - 0.00025).abs() < 1e-9);
    }

    #[test]
    fn compute_lr_constant_ignores_epoch() {
        assert_eq!(
            compute_lr(LrSchedule::Constant, 0.001, 0.0, 1, 20, 0),
            0.001
        );
        assert_eq!(
            compute_lr(LrSchedule::Constant, 0.001, 0.0, 20, 20, 0),
            0.001
        );
    }

    #[test]
    fn compute_lr_min_lr_floors_step_half_too() {
        // By epoch 20, unfloored step-half is ~1.9e-9 -- min_lr must clamp it up.
        let lr = compute_lr(LrSchedule::StepHalf, 0.001, 0.0001, 20, 20, 0);
        assert_eq!(lr, 0.0001);
    }

    #[test]
    fn compute_lr_cosine_starts_at_base_and_ends_at_min_lr_exactly() {
        let first = compute_lr(LrSchedule::Cosine, 0.001, 0.00001, 1, 20, 0);
        let last = compute_lr(LrSchedule::Cosine, 0.001, 0.00001, 20, 20, 0);
        assert!((first - 0.001).abs() < 1e-9);
        assert_eq!(last, 0.00001);
    }

    #[test]
    fn compute_lr_warmup_ramps_linearly_and_lands_on_base_lr() {
        let half = compute_lr(LrSchedule::StepHalf, 0.001, 0.0, 2, 20, 4);
        assert!((half - 0.0005).abs() < 1e-9); // epoch 2/4 warmup = 50% of base_lr
        let at_boundary = compute_lr(LrSchedule::StepHalf, 0.001, 0.0, 4, 20, 4);
        assert!((at_boundary - 0.001).abs() < 1e-9); // epoch == warmup_epochs -> exactly base_lr
        let first_post_warmup = compute_lr(LrSchedule::StepHalf, 0.001, 0.0, 5, 20, 4);
        assert!((first_post_warmup - 0.001).abs() < 1e-9); // decay restarts fresh from base_lr
    }

    #[test]
    fn compute_lr_single_epoch_run_uses_base_lr_for_every_schedule() {
        assert_eq!(compute_lr(LrSchedule::StepHalf, 0.001, 0.0, 1, 1, 0), 0.001);
        assert_eq!(compute_lr(LrSchedule::Constant, 0.001, 0.0, 1, 1, 0), 0.001);
        assert!((compute_lr(LrSchedule::Cosine, 0.001, 0.0, 1, 1, 0) - 0.001).abs() < 1e-9);
    }

    #[test]
    fn compute_lr_warmup_equals_total_epochs_never_panics() {
        // Every epoch falls inside the warmup window -- the post-warmup
        // divide-by-zero guard must never actually be exercised, but the
        // whole range must still compute without panicking.
        for epoch in 1..=5u32 {
            let lr = compute_lr(LrSchedule::Cosine, 0.001, 0.0, epoch, 5, 5);
            assert!(lr.is_finite() && lr >= 0.0);
        }
        assert_eq!(compute_lr(LrSchedule::Cosine, 0.001, 0.0, 5, 5, 5), 0.001);
    }

    #[test]
    fn lr_schedule_parse_roundtrips_known_names_and_rejects_unknown() {
        assert_eq!(LrSchedule::parse("constant"), Some(LrSchedule::Constant));
        assert_eq!(LrSchedule::parse("step-half"), Some(LrSchedule::StepHalf));
        assert_eq!(LrSchedule::parse("cosine"), Some(LrSchedule::Cosine));
        assert_eq!(LrSchedule::parse("bogus"), None);
    }

    #[test]
    fn position_teacher_reuses_cached_search_on_repeated_position() {
        let mut trainer = Trainer::new(1);
        let mut cache: HashMap<String, i32> = HashMap::new();
        let mut board = Board::startpos();

        let first = trainer.position_teacher(&mut board, GameResult::Unknown, 2, None, &mut cache);
        assert_eq!(trainer.cache_misses, 1);
        assert_eq!(trainer.cache_hits, 0);
        assert_eq!(cache.len(), 1);

        let mut board_again = Board::startpos();
        let second =
            trainer.position_teacher(&mut board_again, GameResult::Unknown, 2, None, &mut cache);
        assert_eq!(trainer.cache_misses, 1, "second call must not re-search");
        assert_eq!(trainer.cache_hits, 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(first, second);
    }
}
