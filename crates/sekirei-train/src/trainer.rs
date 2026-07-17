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
use crate::diagnostics;

/// The sampled position's own game-result signal, on the same ±600
/// centipawn scale as a clamped eval teacher (loss=-600, draw=0, win=+600),
/// from `stm`'s perspective. `None` for `GameResult::Unknown` -- there is no
/// win/draw/loss signal to give for an aborted/timed-out/illegal-move game,
/// and guessing one (e.g. treating it as a draw) would add noise instead of
/// signal (see `csa.rs`'s `GameResult` doc).
/// `scale` (default `1200.0`, giving `±600` -- see `--wdl-target-scale`)
/// is the only place `wdl_target`'s native range lives. Investigated as a
/// lever after `docs/experiments/cp_wdl_target_residual_trace.md` found
/// this fixed-per-game-result value structurally dominates the blended
/// gradient over the fine-grained per-position `eval_teacher`, regardless
/// of `--wdl-lambda`'s weighting.
fn wdl_target_cp(result: GameResult, stm: Color, scale: f32) -> Option<f32> {
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
    Some((wdl - 0.5) * scale)
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

/// Which layer's own Adam update to skip under `--diagnostic-freeze-layer`
/// (see `Trainer::diagnostic_freeze_layer`'s doc comment) -- a causal probe
/// for `docs/experiments/l2_saturation_mechanism_p0.md`'s correlated FT+L2
/// finding, not a training feature. Only one layer at a time is supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezeLayer {
    Ft,
    L2,
    Out,
}

impl FreezeLayer {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ft" => Some(FreezeLayer::Ft),
            "l2" => Some(FreezeLayer::L2),
            "out" => Some(FreezeLayer::Out),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FreezeLayer::Ft => "ft",
            FreezeLayer::L2 => "l2",
            FreezeLayer::Out => "out",
        }
    }
}

/// Computes the learning rate for `epoch` (1-indexed) against a schedule
/// shaped for `total_epochs`. `total_epochs` is the schedule's *horizon* --
/// how long a run the curve is shaped for -- not necessarily how many
/// epochs the caller actually runs (see `resolve_schedule_epochs`, which
/// callers should use to derive this value from `--epochs`/
/// `--lr-schedule-epochs`). Because this function is pure and never sees
/// "how many epochs will actually run," a short run and a long run that
/// pass the same `total_epochs` always agree epoch-for-epoch on every
/// epoch they share -- there is no way for them to diverge before the
/// point where `total_epochs` itself would have been exceeded.
///
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

/// Resolves `--lr-schedule-epochs` against `--epochs`, for reproducing the
/// first N epochs of a longer schedule (e.g. `--epochs 3
/// --lr-schedule-epochs 20` shapes the LR curve for a 20-epoch run but only
/// executes epochs 1-3 of it) without changing today's default behavior.
///
/// `requested = None` means the flag was omitted -- defaults to `epochs`,
/// reproducing the pre-existing behavior exactly (schedule horizon ==
/// actual run length). `epochs == 0` (the `--epochs 0` trick for dumping an
/// untrained checkpoint) is passed straight through with no validation --
/// the epoch loop never runs and `compute_lr` is never called, so no
/// schedule value can be wrong. Otherwise errors rather than silently
/// clamping on:
/// - `schedule_epochs == 0` (a zero-length schedule is meaningless)
/// - `warmup_epochs > schedule_epochs` (warmup would never complete)
/// - `schedule_epochs < epochs` (the run would run past the schedule's
///   horizon, hitting undefined "continue past the end" behavior --
///   `compute_lr` currently just holds at the final epoch's value, but
///   that's almost never the intent, so surface the mistake instead of
///   quietly clamping the run length or the horizon)
pub fn resolve_schedule_epochs(
    epochs: u32,
    requested: Option<u32>,
    warmup_epochs: u32,
) -> Result<u32, String> {
    if epochs == 0 {
        return Ok(requested.unwrap_or(0));
    }
    let schedule_epochs = requested.unwrap_or(epochs);
    if schedule_epochs == 0 {
        return Err("--lr-schedule-epochs must be greater than 0".to_string());
    }
    if warmup_epochs > schedule_epochs {
        return Err(format!(
            "--warmup-epochs ({warmup_epochs}) cannot exceed --lr-schedule-epochs ({schedule_epochs})"
        ));
    }
    if schedule_epochs < epochs {
        return Err(format!(
            "--lr-schedule-epochs ({schedule_epochs}) cannot be less than --epochs ({epochs}) -- \
             use a schedule horizon at least as long as the run, or omit the flag to default it to --epochs"
        ));
    }
    Ok(schedule_epochs)
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

/// Deterministic Fisher-Yates shuffle of `0..n`, for `--shuffle-seed`.
/// Reuses the same `Lcg` weight-init already uses -- no new PRNG needed.
pub fn shuffled_order(n: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    let mut rng = Lcg(seed ^ 0xD1B5_4A32_D192_ED03);
    for i in (1..n).rev() {
        // `next_u64() % (i+1)` has a small modulo bias, negligible at
        // dataset-shuffle scale (not a cryptographic or statistical-test
        // use) and consistent with `Lcg::uniform`'s own bias tradeoff.
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order
}

// ---- Training weight container ----

#[derive(Clone)]
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
    pub fn new_seeded(seed: u64, l2_bias_init: f32) -> Self {
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
            // permanently blocking gradient flow to l2/ft. `l2_bias_init` (default 0.5,
            // see --l2-bias-init) lets this be tuned against the actual He-init spread
            // instead of staying at the value that only ever had to clear zero.
            l2_bias: vec![l2_bias_init; L2],
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

    /// Inverse of `to_nnue_weights` -- loads an already-trained checkpoint
    /// for further forward-pass use (e.g. `--eval-only`'s common-metric
    /// back-apply). Adam moments start at zero and `step` at 0: nothing
    /// resumes training from here, so there's no prior optimizer state to
    /// restore (checkpoints are inference-only weight files -- Adam moments
    /// were never persisted in the first place).
    pub fn from_nnue_weights(w: &NnueWeights) -> Self {
        const FT_SCALE: f32 = 64.0;
        let ft_len = INPUT * L1;
        let l2_len = 2 * L1 * L2;
        let out_len = L2;

        let mut ft = vec![0.0f32; ft_len];
        for i in 0..INPUT {
            for j in 0..L1 {
                ft[i * L1 + j] = w.ft[i][j] as f32 / FT_SCALE;
            }
        }
        let ft_bias: Vec<f32> = w.ft_bias.iter().map(|&v| v as f32 / FT_SCALE).collect();

        let mut l2 = vec![0.0f32; l2_len];
        for i in 0..2 * L1 {
            for o in 0..L2 {
                l2[i * L2 + o] = w.l2[i][o];
            }
        }

        TrainWeights {
            ft,
            ft_bias,
            l2,
            l2_bias: w.l2_bias.to_vec(),
            out: w.out.to_vec(),
            out_bias: w.out_bias,

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

    /// Raw L2 weight matrix, row-major `2*L1 × L2` -- for
    /// `diagnostics::l2_row_weight_norm_per_neuron`.
    pub fn l2(&self) -> &[f32] {
        &self.l2
    }

    /// Raw L2 bias vector, length `L2`.
    pub fn l2_bias(&self) -> &[f32] {
        &self.l2_bias
    }

    /// Raw output-layer weight vector, length `L2` -- for
    /// `diagnostics::output_weight_norm`.
    pub fn out(&self) -> &[f32] {
        &self.out
    }

    /// Output-layer bias (scalar).
    pub fn out_bias(&self) -> f32 {
        self.out_bias
    }
}

// ---- Trainer ----

/// Per-game validation accumulator returned by `Trainer::eval_game`,
/// folded across a validation set's games. `cp_mse_sum`/`wdl_loss_sum` are
/// computed against the raw teacher components regardless of the run's own
/// `wdl_lambda` -- the common yardstick that makes `valid_cp_mse` (mean of
/// `cp_mse_sum/count`) comparable across runs trained at different λ,
/// unlike `loss_sum/count` which is only comparable within one λ.
#[derive(Debug, Clone, Copy)]
pub struct ValidStats {
    pub loss_sum: f64,
    pub count: u64,
    pub cp_mse_sum: f64,
    pub wdl_loss_sum: f64,
    pub wdl_count: u64,
    pub output_sum: f64,
    pub output_sum_sq: f64,
    // `mean_std`'s variance formula (sum_sq/n - mean^2) hits catastrophic
    // cancellation near-constant output and its `max(0.0)` guard can round
    // a genuinely non-zero std down to an exact 0.000 -- min/max/range are
    // computed directly with no cancellation, so `range == 0.0` means truly
    // constant output and a small nonzero range means "collapsed but not
    // literally frozen." Identity element for `Add`/fold is (+inf, -inf),
    // not (0.0, 0.0) -- see `Default` below.
    pub output_min: f32,
    pub output_max: f32,
}

impl Default for ValidStats {
    fn default() -> Self {
        ValidStats {
            loss_sum: 0.0,
            count: 0,
            cp_mse_sum: 0.0,
            wdl_loss_sum: 0.0,
            wdl_count: 0,
            output_sum: 0.0,
            output_sum_sq: 0.0,
            output_min: f32::INFINITY,
            output_max: f32::NEG_INFINITY,
        }
    }
}

impl std::ops::Add for ValidStats {
    type Output = ValidStats;
    fn add(self, other: ValidStats) -> ValidStats {
        ValidStats {
            loss_sum: self.loss_sum + other.loss_sum,
            count: self.count + other.count,
            cp_mse_sum: self.cp_mse_sum + other.cp_mse_sum,
            wdl_loss_sum: self.wdl_loss_sum + other.wdl_loss_sum,
            wdl_count: self.wdl_count + other.wdl_count,
            output_sum: self.output_sum + other.output_sum,
            output_sum_sq: self.output_sum_sq + other.output_sum_sq,
            output_min: self.output_min.min(other.output_min),
            output_max: self.output_max.max(other.output_max),
        }
    }
}

pub struct Trainer {
    pub weights: TrainWeights,
    pub total_loss: f64,
    pub total_count: u64,
    pub total_weight: f64,    // sum of weights (for avg_weight log)
    pub dropped_missing: u64, // positions skipped (not in scored map)
    pub lr: f32,
    // Global (whole-network) gradient-norm clip threshold -- `None` (the
    // default) means no clipping, byte-identical to pre-clipping behavior.
    // Run-level config, not reset by `reset_epoch_stats`, same as `lr`.
    // Scales all layers' gradients down together (preserving direction)
    // when the pre-clip global norm exceeds this, applied *after* the
    // gradient-norm diagnostics above capture the unclipped value -- so
    // the diagnostic always reflects the natural distribution regardless
    // of whether clipping is active, letting a clip threshold be chosen
    // from a run's own diagnostic output.
    pub grad_clip_norm: Option<f32>,
    pub grad_clip_count: u64,
    // Per-layer clip thresholds -- independent of `grad_clip_norm` above and
    // of each other: each layer's gradient is compared against *its own*
    // norm and *its own* threshold (not the combined global norm), and only
    // that layer's gradient is scaled if it's exceeded. `None` (the default
    // for all three) means that layer is never touched. Applied *before*
    // `grad_clip_norm`'s global-norm check, so setting only `out_clip_norm`
    // (the 2026-07-15 output-only-clipping experiment) leaves FT/L2
    // completely untouched -- a real single-variable change, not global
    // clipping with FT/L2 thresholds set very high.
    pub ft_clip_norm: Option<f32>,
    pub l2_clip_norm: Option<f32>,
    pub out_clip_norm: Option<f32>,
    pub ft_clip_count: u64,
    pub l2_clip_count: u64,
    pub out_clip_count: u64,
    // Per-position output-layer gradient norm, captured unconditionally
    // (like `global_grad_norm_values`) so a per-layer clip threshold can be
    // chosen from a run's own percentile output rather than reusing
    // `grad_clip_norm`'s global-norm-derived value, which is dominated by
    // whichever layer has the largest raw scale (empirically `out` itself,
    // so the two distributions are related but not the same thing).
    pub out_grad_norm_values: Vec<f32>,
    // Mean/std of the output-layer gradient norm *after* per-layer clipping
    // is applied -- alongside `out_grad_norm_sum`/`sum_sq` above (which,
    // like the diagnostics elsewhere in this struct, stay pre-clip), this
    // pair shows how much clipping actually moved the distribution.
    pub out_grad_norm_after_sum: f64,
    pub out_grad_norm_after_sum_sq: f64,
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
    // Frequency-based L2 diagnostics -- per-sample counts, distinct from
    // the ever-flags above (see `diagnostics.rs`'s `l2_dead_neurons` doc
    // comment for why "ever active" and "always active" are different
    // questions). `l2_values` holds every sample's raw pre-clamp L2 value
    // per neuron for percentile computation.
    //
    // ponytail: `l2_values` is O(epoch samples × L2) memory (one epoch's
    // worth of f32s); fine at this dataset's scale, switch to a streaming
    // quantile sketch if that changes.
    pub l2_zero_count: Vec<u64>,
    pub l2_sat_count: Vec<u64>,
    pub l2_sample_count: u64,
    pub l2_values: Vec<Vec<f32>>,
    // Per-position gradient-norm diagnostics, one accumulator triple per
    // layer (mean/std via sum + sum_sq, matching `output_sum`'s pattern).
    // "Layer" bundles a weight matrix with its bias (e.g. FT = `ft` +
    // `ft_bias`). Distinct from update-norm below: under Adam, a smaller
    // gradient doesn't imply a smaller applied step (√v̂ normalizes scale
    // out), so gradient norm alone can't answer "does λ just shrink the
    // gradient" -- update norm is the complementary signal for that.
    pub ft_grad_norm_sum: f64,
    pub ft_grad_norm_sum_sq: f64,
    pub l2_grad_norm_sum: f64,
    pub l2_grad_norm_sum_sq: f64,
    pub out_grad_norm_sum: f64,
    pub out_grad_norm_sum_sq: f64,
    // Global (whole-network) gradient norm, one entry per position --
    // full capture (not just sum/sum_sq) because picking a gradient-clip
    // threshold needs percentiles (p95/p99), not just a mean.
    //
    // ponytail: O(epoch samples) memory, same tradeoff as `l2_values`.
    pub global_grad_norm_values: Vec<f32>,
    // Per-position *applied* update norm per layer -- the actual step
    // Adam takes, as opposed to the raw gradient magnitude above.
    pub ft_update_norm_sum: f64,
    pub ft_update_norm_sum_sq: f64,
    pub l2_update_norm_sum: f64,
    pub l2_update_norm_sum_sq: f64,
    pub out_update_norm_sum: f64,
    pub out_update_norm_sum_sq: f64,
    // Target/prediction distribution and their relationship. `target_*`
    // is the blended teacher actually trained against (within-run
    // monitoring only -- not comparable across `wdl_lambda`). The
    // prediction-target correlation instead uses the *raw* eval component
    // (`eval_teacher_*`/`pred_eval_prod_sum`), so it stays comparable
    // across runs at different λ, matching `valid_cp_mse`'s rationale.
    pub target_sum: f64,
    pub target_sum_sq: f64,
    pub eval_teacher_sum: f64,
    pub eval_teacher_sum_sq: f64,
    pub pred_eval_prod_sum: f64,
    // Training-side CP/WDL loss components, computed against the same raw
    // components as `ValidStats` (see `position_teacher_components`) but
    // never used for the actual gradient -- purely diagnostic, so a run's
    // reported total training loss and its optimization target are
    // unchanged. Answers "is λ=0.7 a genuinely better-fitting auxiliary
    // signal, or just a smaller/smoother objective that masks a worse
    // cp fit" -- total_loss alone can't distinguish those.
    pub cp_component_sum: f64,
    pub wdl_component_sum: f64,
    pub wdl_component_count: u64,
    // CSA-path teacher-search cache counters (see `position_teacher`).
    // Reset every epoch by `reset_epoch_stats`, same as the diagnostics
    // above.
    pub cache_hits: u64,
    pub cache_misses: u64,

    // ---- Epoch-1 batch-level trace (--trace-positions) ----
    // Run-level config, not reset by `reset_epoch_stats`, same as `lr`.
    // Position-counts (since epoch start, 1-indexed by `l2_sample_count`
    // after each `train_position` call) at which to snapshot the
    // accumulators below. Empty means the feature is off -- the
    // accumulators are still maintained (cheap counters/sums), but nothing
    // is ever pushed to `trace_snapshots` and no `.trace.json` is written.
    // Requesting `0` is a harmless no-op (the first snapshot opportunity is
    // after position 1 completes) -- the pre-training state is already
    // available via the existing `--epochs 0` + `l2_saturation_probe`
    // methodology, not something this trace needs to duplicate.
    pub trace_positions: std::collections::HashSet<u64>,
    // One entry per snapshot taken so far this epoch. Reset every epoch by
    // `reset_epoch_stats`, consumed (written to `.trace.json`) by the
    // caller at epoch end, same lifecycle as `l2_values` etc.
    pub trace_snapshots: Vec<diagnostics::TraceSnapshot>,
    // Opt-in (`--trace-weights`, default off): at each `trace_positions`
    // marker, additionally clone the full raw f32 `TrainWeights` (not the
    // quantized `NnueWeights` epoch-end checkpoints use) into
    // `weight_snapshots` -- feeds the P0b forward-side Δz decomposition
    // (`docs/experiments/`'s epoch-1 zero-gradient-collapse investigation),
    // which needs FT output computed exactly as training itself computes
    // it, not through i16 quantization noise. Off by default: cloning full
    // weights (a few MB) at every trace point is real cost, unlike the
    // existing aggregate `TraceSnapshot`.
    pub weight_snapshot_trace: bool,
    pub weight_snapshots: Vec<(u64, TrainWeights)>,
    // Per-neuron accumulators for the trace, all epoch-scoped (reset in
    // `reset_epoch_stats`, never mid-epoch -- a snapshot reads these
    // cumulative-since-epoch-start, the same semantic the existing
    // epoch-end diagnostics already use for `l2_values`/`l2_zero_count`).
    //
    // `l2_weighted_input_values[o]` is `l2_values[o]` minus that sample's
    // `l2_bias[o]` -- the two terms of `L2_preactivation = FT_output ×
    // L2_weight + L2_bias` (see `train_position`), so a trace can tell
    // whether a neuron's pre-activation moved because its incoming weights
    // /FT input moved, or because its own bias moved.
    pub l2_weighted_input_values: Vec<Vec<f32>>,
    // `d_l2_acc[o]`/`d_bias[j]` (see `train_position`'s backward pass) are
    // the gradient of the loss w.r.t. that neuron's own pre-activation --
    // the most direct answer to "which wall is this neuron being pushed
    // toward this step", more direct than aggregating incoming
    // weight-gradients would be. Sum/sum-of-squares for mean/norm,
    // pos/neg counts for sign consistency (`|pos-neg|/(pos+neg)`).
    pub l2_dacc_sum: Vec<f64>,
    pub l2_dacc_sq_sum: Vec<f64>,
    pub l2_dacc_pos_count: Vec<u64>,
    pub l2_dacc_neg_count: Vec<u64>,
    pub ft_dacc_sum: Vec<f64>,
    pub ft_dacc_sq_sum: Vec<f64>,
    pub ft_dacc_pos_count: Vec<u64>,
    pub ft_dacc_neg_count: Vec<u64>,
    // Per-neuron applied-Adam-update norm, from the bias parameter only
    // (not the incoming weight rows -- L2's are dense but strided, FT's
    // are sparse over `active_features`; the bias update is already
    // exactly one element per neuron for both layers, the cheapest
    // available per-neuron signal for "how much is this neuron's
    // threshold moving").
    pub l2_bias_update_sq_sum: Vec<f64>,
    pub ft_bias_update_sq_sum: Vec<f64>,
    // FT's existing `ft_ever_active`/`ft_ever_saturated` are "ever this
    // epoch" booleans, not frequencies -- mirrors L2's
    // `l2_zero_count`/`l2_sat_count`/`l2_sample_count` so FT reaches the
    // same frequency-based granularity. "Dead" mirrors `ft_ever_active`'s
    // OR-across-perspectives convention negated (neither side fires);
    // "saturated" mirrors `ft_ever_saturated`'s OR convention directly
    // (either side saturates).
    pub ft_zero_count: Vec<u64>,
    pub ft_sat_count: Vec<u64>,
    // Norm of the concatenated 2×L1-wide FT-output vector feeding L2 for
    // each position (`relu_us`/`relu_them`) -- not per-neuron, one pair of
    // running sums for mean/std across positions processed so far.
    pub l2_input_norm_sum: f64,
    pub l2_input_norm_sq_sum: f64,
    // Mean/std of FT's own post-activation output -- pooled across both
    // perspectives and all L1 neurons (not per-neuron, layer-wide, same
    // shape as `l2_input_norm_*` above), across positions so far.
    pub ft_output_sum: f64,
    pub ft_output_sum_sq: f64,
    pub ft_output_count: u64,

    // ---- CP/WDL gradient decomposition (--cp-wdl-grad-trace) ----
    // Run-level config, not reset by `reset_epoch_stats`, same as `lr`.
    // Off by default -- `diagnostic_backward` (see its doc comment) is
    // simply never called when this is `false`, zero added cost to
    // ordinary training. Only meaningful where `wdl_target` is `Some`
    // (CSA path with `--wdl-lambda` set); positions without a WDL signal
    // are skipped for this diagnostic, same gating `wdl_component_count`
    // already uses.
    pub cp_wdl_grad_trace: bool,
    // Per-neuron L2/FT gradient accumulators, one pair per teacher signal,
    // same shape as `l2_dacc_sum`/`ft_dacc_sum` above (mean/RMS/sign-
    // consistency) but computed from `err_cp`/`err_wdl` alone instead of
    // the blended `err`. `{l2,ft}_cp_wdl_dot_sum` is the running dot
    // product between the two signals' per-position gradients -- combined
    // with `..._sq_sum` above, gives per-neuron cosine similarity
    // (`dot_sum / sqrt(cp_sq_sum * wdl_sq_sum)`) without storing full
    // per-position history.
    pub l2_cp_dacc_sum: Vec<f64>,
    pub l2_cp_dacc_sq_sum: Vec<f64>,
    pub l2_cp_dacc_pos_count: Vec<u64>,
    pub l2_cp_dacc_neg_count: Vec<u64>,
    pub l2_wdl_dacc_sum: Vec<f64>,
    pub l2_wdl_dacc_sq_sum: Vec<f64>,
    pub l2_wdl_dacc_pos_count: Vec<u64>,
    pub l2_wdl_dacc_neg_count: Vec<u64>,
    pub l2_cp_wdl_dot_sum: Vec<f64>,
    pub ft_cp_dacc_sum: Vec<f64>,
    pub ft_cp_dacc_sq_sum: Vec<f64>,
    pub ft_cp_dacc_pos_count: Vec<u64>,
    pub ft_cp_dacc_neg_count: Vec<u64>,
    pub ft_wdl_dacc_sum: Vec<f64>,
    pub ft_wdl_dacc_sq_sum: Vec<f64>,
    pub ft_wdl_dacc_pos_count: Vec<u64>,
    pub ft_wdl_dacc_neg_count: Vec<u64>,
    pub ft_cp_wdl_dot_sum: Vec<f64>,
    // Whole-layer gradient-norm mean/std, one pair per (layer, signal) --
    // same shape as `ft_grad_norm_sum`/`l2_grad_norm_sum`/
    // `out_grad_norm_sum` above, split by CP-only vs. WDL-only instead of
    // blended. Denominator for mean/std is `wdl_component_count` (already
    // tracks exactly "positions this diagnostic ran on").
    pub cp_ft_grad_norm_sum: f64,
    pub cp_ft_grad_norm_sum_sq: f64,
    pub wdl_ft_grad_norm_sum: f64,
    pub wdl_ft_grad_norm_sum_sq: f64,
    pub cp_l2_grad_norm_sum: f64,
    pub cp_l2_grad_norm_sum_sq: f64,
    pub wdl_l2_grad_norm_sum: f64,
    pub wdl_l2_grad_norm_sum_sq: f64,
    pub cp_out_grad_norm_sum: f64,
    pub cp_out_grad_norm_sum_sq: f64,
    pub wdl_out_grad_norm_sum: f64,
    pub wdl_out_grad_norm_sum_sq: f64,
    // Target/prediction/residual/dL-dOutput distributions, scoped to the
    // same wdl-having position subset the CP/WDL gradient fields above
    // already use (not the epoch-wide `eval_teacher_sum`/`output_sum`,
    // which include positions this diagnostic never touches) -- so every
    // field here is a fair, apples-to-apples comparison over the identical
    // position set. Explains *why* the CP/WDL gradient scales differ
    // (target/prediction/residual magnitude), not just that they do.
    pub cp_target_sum: f64,
    pub cp_target_sum_sq: f64,
    pub wdl_target_sum: f64,
    pub wdl_target_sum_sq: f64,
    pub prediction_sum: f64,
    pub prediction_sum_sq: f64,
    // Signed residual (score - target), distinct from `cp_component_sum`/
    // `wdl_component_sum` above, which are squared-error (MSE) sums --
    // those can't distinguish "consistently offset" from "large but
    // symmetric" error, which is exactly what motivates the gradient-scale
    // question this trace answers.
    pub cp_residual_sum: f64,
    pub cp_residual_sum_sq: f64,
    pub wdl_residual_sum: f64,
    pub wdl_residual_sum_sq: f64,
    // `d_output` (gradient of the loss w.r.t. the network's scalar output)
    // per signal -- `diagnostic_backward` already computes this internally
    // for both CP-only and WDL-only `err`, previously discarded.
    pub cp_d_output_sum: f64,
    pub cp_d_output_sum_sq: f64,
    pub wdl_d_output_sum: f64,
    pub wdl_d_output_sum_sq: f64,

    // ---- Per-sample gradient correlation trace (--sample-grad-trace) ----
    // Run-level config, not reset by `reset_epoch_stats`, same as `lr`.
    // Stage 1 of the "does epoch-1's gradient accumulate in one direction"
    // investigation (`docs/experiments/wdl_target_scale_ablation.md`'s
    // deferred next-direction note) -- records one `SampleGradRecord` per
    // position, up to this many positions per epoch, *without* changing
    // training order or applying any update the flag wouldn't otherwise
    // apply. `0` (default) means off: no records pushed, no extra compute
    // beyond the two cheap scalar `d_output` values already derivable from
    // `eval_teacher`/`wdl_target` (no full `diagnostic_backward` call
    // needed for those, unlike `--cp-wdl-grad-trace`). Reordering the
    // recorded samples offline (game-shuffled / sample-shuffled / outcome-
    // balanced) is Stage 2, done by a separate analysis script against this
    // trace's JSONL output -- this flag alone never reorders anything.
    pub sample_grad_trace_limit: u64,
    // One entry per position recorded so far this epoch. Reset every epoch
    // by `reset_epoch_stats`, consumed (written to
    // `.epochN.sample_grad.jsonl`) by the caller at epoch end.
    pub sample_grad_records: Vec<diagnostics::SampleGradRecord>,
    // Previous recorded sample's raw 32-wide `d_l2_acc` vector, for
    // `cosine_prev`. `None` for the first recorded sample of the epoch (no
    // previous vector to compare against) and after `reset_epoch_stats`.
    sample_grad_prev_d_l2_acc: Option<[f32; L2]>,
    // Running arithmetic mean of `d_l2_acc` across all recorded samples so
    // far this epoch, updated incrementally (`mean += (x - mean) / count`)
    // so no per-position history needs to be kept just for this. Reset
    // (zeroed, alongside `sample_grad_running_count`) every epoch.
    sample_grad_running_mean_d_l2_acc: [f32; L2],
    sample_grad_running_count: u64,

    // ---- Freeze diagnostic (--diagnostic-freeze-layer / --diagnostic-freeze-from-position / --diagnostic-freeze-until-position) ----
    // Run-level config, not reset by `reset_epoch_stats`, same as `lr`.
    // `None` (default) means no freezing -- byte-identical to this flag
    // never having existed. When set, the named layer's own Adam update
    // (its params *and* its `m`/`v` moments) is skipped entirely for as
    // long as `diagnostic_freeze_from_position <= l2_sample_count <=
    // diagnostic_freeze_until_position` this epoch -- a closed window, not
    // just an from-the-start cutoff (`from_position` defaults to `0`, which
    // is always `<= l2_sample_count`, so the original "freeze from the very
    // first position" behavior is exactly `from_position=0`, unchanged).
    // This is NOT stop-gradient: the ordinary backward pass above already
    // computes every layer's gradient through this layer's *current*
    // (frozen) weight values before the freeze gate is checked, so
    // upstream/downstream layers still receive a real gradient signal
    // through it -- only this layer's own parameter update is discarded.
    // A causal probe for `docs/experiments/l2_saturation_mechanism_p0.md`'s
    // correlated FT+L2 co-movement finding (which the frozen-weights Δz
    // decomposition there could show but not prove causally), not a
    // training improvement. Multiple simultaneous frozen layers are not
    // supported (single `Option`, not a set) -- not needed for the first
    // pass at isolating which single layer's movement is necessary.
    pub diagnostic_freeze_layer: Option<FreezeLayer>,
    pub diagnostic_freeze_from_position: u64,
    pub diagnostic_freeze_until_position: u64,

    searcher: Searcher,
}

impl Trainer {
    pub fn new(seed: u64, l2_bias_init: f32) -> Self {
        let tt = Tt::new(4); // Tt::new returns Arc<Tt>
        Trainer {
            weights: TrainWeights::new_seeded(seed, l2_bias_init),
            total_loss: 0.0,
            total_count: 0,
            total_weight: 0.0,
            dropped_missing: 0,
            lr: 0.001,
            grad_clip_norm: None,
            grad_clip_count: 0,
            ft_clip_norm: None,
            l2_clip_norm: None,
            out_clip_norm: None,
            ft_clip_count: 0,
            l2_clip_count: 0,
            out_clip_count: 0,
            out_grad_norm_values: Vec::new(),
            out_grad_norm_after_sum: 0.0,
            out_grad_norm_after_sum_sq: 0.0,
            ft_ever_active: vec![false; L1],
            ft_ever_saturated: vec![false; L1],
            l2_ever_active: vec![false; L2],
            l2_ever_saturated: vec![false; L2],
            output_sum: 0.0,
            output_sum_sq: 0.0,
            l2_zero_count: vec![0; L2],
            l2_sat_count: vec![0; L2],
            l2_sample_count: 0,
            l2_values: vec![Vec::new(); L2],
            ft_grad_norm_sum: 0.0,
            ft_grad_norm_sum_sq: 0.0,
            l2_grad_norm_sum: 0.0,
            l2_grad_norm_sum_sq: 0.0,
            out_grad_norm_sum: 0.0,
            out_grad_norm_sum_sq: 0.0,
            global_grad_norm_values: Vec::new(),
            ft_update_norm_sum: 0.0,
            ft_update_norm_sum_sq: 0.0,
            l2_update_norm_sum: 0.0,
            l2_update_norm_sum_sq: 0.0,
            out_update_norm_sum: 0.0,
            out_update_norm_sum_sq: 0.0,
            target_sum: 0.0,
            target_sum_sq: 0.0,
            eval_teacher_sum: 0.0,
            eval_teacher_sum_sq: 0.0,
            pred_eval_prod_sum: 0.0,
            cp_component_sum: 0.0,
            wdl_component_sum: 0.0,
            wdl_component_count: 0,
            cache_hits: 0,
            cache_misses: 0,
            trace_positions: std::collections::HashSet::new(),
            trace_snapshots: Vec::new(),
            weight_snapshot_trace: false,
            weight_snapshots: Vec::new(),
            l2_weighted_input_values: vec![Vec::new(); L2],
            l2_dacc_sum: vec![0.0; L2],
            l2_dacc_sq_sum: vec![0.0; L2],
            l2_dacc_pos_count: vec![0; L2],
            l2_dacc_neg_count: vec![0; L2],
            ft_dacc_sum: vec![0.0; L1],
            ft_dacc_sq_sum: vec![0.0; L1],
            ft_dacc_pos_count: vec![0; L1],
            ft_dacc_neg_count: vec![0; L1],
            l2_bias_update_sq_sum: vec![0.0; L2],
            ft_bias_update_sq_sum: vec![0.0; L1],
            ft_zero_count: vec![0; L1],
            ft_sat_count: vec![0; L1],
            l2_input_norm_sum: 0.0,
            l2_input_norm_sq_sum: 0.0,
            ft_output_sum: 0.0,
            ft_output_sum_sq: 0.0,
            ft_output_count: 0,
            cp_wdl_grad_trace: false,
            l2_cp_dacc_sum: vec![0.0; L2],
            l2_cp_dacc_sq_sum: vec![0.0; L2],
            l2_cp_dacc_pos_count: vec![0; L2],
            l2_cp_dacc_neg_count: vec![0; L2],
            l2_wdl_dacc_sum: vec![0.0; L2],
            l2_wdl_dacc_sq_sum: vec![0.0; L2],
            l2_wdl_dacc_pos_count: vec![0; L2],
            l2_wdl_dacc_neg_count: vec![0; L2],
            l2_cp_wdl_dot_sum: vec![0.0; L2],
            ft_cp_dacc_sum: vec![0.0; L1],
            ft_cp_dacc_sq_sum: vec![0.0; L1],
            ft_cp_dacc_pos_count: vec![0; L1],
            ft_cp_dacc_neg_count: vec![0; L1],
            ft_wdl_dacc_sum: vec![0.0; L1],
            ft_wdl_dacc_sq_sum: vec![0.0; L1],
            ft_wdl_dacc_pos_count: vec![0; L1],
            ft_wdl_dacc_neg_count: vec![0; L1],
            ft_cp_wdl_dot_sum: vec![0.0; L1],
            cp_ft_grad_norm_sum: 0.0,
            cp_ft_grad_norm_sum_sq: 0.0,
            wdl_ft_grad_norm_sum: 0.0,
            wdl_ft_grad_norm_sum_sq: 0.0,
            cp_l2_grad_norm_sum: 0.0,
            cp_l2_grad_norm_sum_sq: 0.0,
            wdl_l2_grad_norm_sum: 0.0,
            wdl_l2_grad_norm_sum_sq: 0.0,
            cp_out_grad_norm_sum: 0.0,
            cp_out_grad_norm_sum_sq: 0.0,
            wdl_out_grad_norm_sum: 0.0,
            wdl_out_grad_norm_sum_sq: 0.0,
            cp_target_sum: 0.0,
            cp_target_sum_sq: 0.0,
            wdl_target_sum: 0.0,
            wdl_target_sum_sq: 0.0,
            prediction_sum: 0.0,
            prediction_sum_sq: 0.0,
            cp_residual_sum: 0.0,
            cp_residual_sum_sq: 0.0,
            wdl_residual_sum: 0.0,
            wdl_residual_sum_sq: 0.0,
            cp_d_output_sum: 0.0,
            cp_d_output_sum_sq: 0.0,
            wdl_d_output_sum: 0.0,
            wdl_d_output_sum_sq: 0.0,
            sample_grad_trace_limit: 0,
            sample_grad_records: Vec::new(),
            sample_grad_prev_d_l2_acc: None,
            sample_grad_running_mean_d_l2_acc: [0.0; L2],
            sample_grad_running_count: 0,
            diagnostic_freeze_layer: None,
            diagnostic_freeze_from_position: 0,
            diagnostic_freeze_until_position: 0,
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
            // No WDL signal on the positions path (positions.jsonl carries
            // no game_result) -- eval_teacher == teacher, no wdl_target.
            // game_id/game_result are meaningless sentinels here too (see
            // `train_position`'s doc comment).
            self.train_position(
                &sample.board,
                teacher,
                weight,
                teacher,
                None,
                0,
                GameResult::Unknown,
            );
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
    /// `eval_teacher` is cached, not any blended result: the same position
    /// can recur in different games with different results, so the WDL
    /// term is always recomputed from this call's own `result`/
    /// side-to-move. Without this, every epoch re-ran a real label-depth
    /// search on every sampled position -- the exact bug `eval_positions`'s
    /// doc comment describes already being fixed once on the positions path.
    ///
    /// Returns the two raw components a caller blends into its own teacher
    /// (`train_game`/`eval_game` both do this inline, rather than through a
    /// shared blending helper, so the raw components are available to pass
    /// through for diagnostics/common cross-`wdl_lambda` metrics): the
    /// clamped search eval (always present) and the WDL game-outcome target
    /// (`None` for `GameResult::Unknown`, which carries no result signal).
    fn position_teacher_components(
        &mut self,
        board: &mut Board,
        result: GameResult,
        label_depth: u32,
        cache: &mut HashMap<String, i32>,
        wdl_target_scale: f32,
    ) -> (f32, Option<f32>) {
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
        (
            eval_teacher,
            wdl_target_cp(result, board.side_to_move, wdl_target_scale),
        )
    }

    /// Train on a single game.  Samples every `sample_every` plies.
    /// `wdl_lambda`: `None` trains on `eval_teacher` alone (default,
    /// backward-compatible). `Some(λ)` blends in the game's own result from
    /// each sampled position's side-to-move perspective, skipping the blend
    /// (falling back to pure eval) for `GameResult::Unknown` games, since
    /// there's no result signal to mix in for those (see `csa.rs`).
    /// `game_id`: the game's stable index into the caller's game list
    /// (e.g. its position in `games: Vec<CsaGame>`, independent of
    /// `--shuffle-seed`'s epoch-order permutation) -- diagnostic-only,
    /// threaded through to `--sample-grad-trace`'s `SampleGradRecord`
    /// alone, never affects training.
    #[allow(clippy::too_many_arguments)]
    pub fn train_game(
        &mut self,
        game_id: u64,
        game: &CsaGame,
        sample_every: usize,
        quiet: bool,
        min_ply: usize,
        label_depth: u32,
        scored: &HashMap<String, f32>,
        stability_weighted: bool,
        wdl_lambda: Option<f32>,
        wdl_target_scale: f32,
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

            // Call `position_teacher_components` directly (rather than the
            // `position_teacher` convenience wrapper) so the raw components
            // are available to thread into `train_position` for diagnostics
            // -- mirrors `eval_game`'s pattern below.
            let (eval_teacher, wdl_target) = self.position_teacher_components(
                &mut board,
                game.result,
                label_depth,
                cache,
                wdl_target_scale,
            );
            let teacher = match (wdl_lambda, wdl_target) {
                (Some(lambda), Some(wdl_target)) => {
                    lambda * eval_teacher + (1.0 - lambda) * wdl_target
                }
                _ => eval_teacher,
            };
            self.train_position(
                &board,
                teacher,
                weight,
                eval_teacher,
                wdl_target,
                game_id,
                game.result,
            );

            board.do_move(mv);
        }
    }

    /// Forward-only pass over a single game for validation loss (no
    /// weight updates, no epoch-stat/diagnostic-counter mutation --
    /// validation measures what training touched, not what validation
    /// itself looked at). Mirrors `train_game`'s replay/sample loop.
    ///
    /// Returns `ValidStats`, not just `(loss_sum, count)`: alongside the
    /// run's own `wdl_lambda`-blended loss, it also accumulates `cp_mse`
    /// (vs. the raw search eval) and `wdl_loss` (vs. the raw game-outcome
    /// target) unconditionally -- the common yardstick that lets runs with
    /// different `wdl_lambda` be compared on the same scale (see
    /// `position_teacher_components`'s doc comment). Free to compute: both
    /// raw components are already produced by the single cached lookup.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn eval_game(
        &mut self,
        game: &CsaGame,
        sample_every: usize,
        quiet: bool,
        min_ply: usize,
        label_depth: u32,
        wdl_lambda: Option<f32>,
        wdl_target_scale: f32,
        cache: &mut HashMap<String, i32>,
    ) -> ValidStats {
        let mut board = Board::startpos();
        let mut stats = ValidStats::default();

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

            let (eval_teacher, wdl_target) = self.position_teacher_components(
                &mut board,
                game.result,
                label_depth,
                cache,
                wdl_target_scale,
            );
            let teacher = match (wdl_lambda, wdl_target) {
                (Some(lambda), Some(wdl_target)) => {
                    lambda * eval_teacher + (1.0 - lambda) * wdl_target
                }
                _ => eval_teacher,
            };
            let score = self.forward(&board);

            let err = (score - teacher) as f64;
            stats.loss_sum += err * err;
            stats.count += 1;

            let cp_err = (score - eval_teacher) as f64;
            stats.cp_mse_sum += cp_err * cp_err;

            if let Some(wdl_target) = wdl_target {
                let wdl_err = (score - wdl_target) as f64;
                stats.wdl_loss_sum += wdl_err * wdl_err;
                stats.wdl_count += 1;
            }

            stats.output_sum += score as f64;
            stats.output_sum_sq += (score as f64) * (score as f64);
            stats.output_min = stats.output_min.min(score);
            stats.output_max = stats.output_max.max(score);

            board.do_move(mv);
        }

        stats
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

    /// One SGD step on a single position. `weight` scales the loss (quietset
    /// stability). `teacher` is the (possibly WDL-blended) value the
    /// gradient is actually computed against -- unchanged from before.
    /// `eval_teacher`/`wdl_target` are the same raw components
    /// `position_teacher_components` produces, threaded through purely for
    /// diagnostics (`cp_component`/`wdl_component`/prediction-eval
    /// correlation below); they never affect the gradient or the weight
    /// update. `train_positions` (no WDL signal available) passes
    /// `eval_teacher = teacher`, `wdl_target = None`. `game_id`/
    /// `game_result` are likewise diagnostic-only, feeding
    /// `--sample-grad-trace`'s `SampleGradRecord` alone -- `train_positions`
    /// (no CSA game to attribute a position to) passes `game_id = 0`,
    /// `game_result = GameResult::Unknown`, meaningless sentinels that only
    /// matter if this trace is ever enabled on that path.
    #[allow(clippy::too_many_arguments)]
    fn train_position(
        &mut self,
        board: &Board,
        teacher: f32,
        weight: f32,
        eval_teacher: f32,
        wdl_target: Option<f32>,
        game_id: u64,
        game_result: GameResult,
    ) {
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
        for &x in relu_us.iter().chain(relu_them.iter()) {
            self.ft_output_sum += x as f64;
            self.ft_output_sum_sq += (x as f64) * (x as f64);
        }
        self.ft_output_count += 2 * L1 as u64;
        for j in 0..L1 {
            if relu_us[j] > 0.0 || relu_them[j] > 0.0 {
                self.ft_ever_active[j] = true;
            }
            if relu_us[j] >= 127.0 || relu_them[j] >= 127.0 {
                self.ft_ever_saturated[j] = true;
            }
            // Frequency-based counterparts of the ever-flags above (see
            // `Trainer::ft_zero_count`'s doc comment): "dead" is the
            // logical complement of `ft_ever_active`'s OR (neither
            // perspective fires this position), "saturated" mirrors
            // `ft_ever_saturated`'s OR directly (either perspective
            // saturates).
            if acc_us[j] <= 0.0 && acc_them[j] <= 0.0 {
                self.ft_zero_count[j] += 1;
            }
            if acc_us[j] >= 127.0 || acc_them[j] >= 127.0 {
                self.ft_sat_count[j] += 1;
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
            let pre = l2_acc[o];
            if pre <= 0.0 {
                self.l2_zero_count[o] += 1;
            }
            if pre >= 127.0 {
                self.l2_sat_count[o] += 1;
            }
            self.l2_values[o].push(pre);
            // `pre` is `weighted_input + l2_bias[o]` (see the accumulation
            // loop above, which starts from `w.l2_bias.clone()`) -- the
            // weighted-input term alone, for `--trace-positions`'s
            // bias-vs-weight-input split.
            self.l2_weighted_input_values[o].push(pre - w.l2_bias[o]);
        }
        self.l2_sample_count += 1;
        let l2_input_norm_sq: f64 = relu_us
            .iter()
            .chain(relu_them.iter())
            .map(|&x| (x as f64).powi(2))
            .sum();
        self.l2_input_norm_sum += l2_input_norm_sq.sqrt();
        self.l2_input_norm_sq_sum += l2_input_norm_sq;

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

        // Diagnostic-only: target/prediction distribution and their
        // relationship, and the loss split into its CP/WDL components --
        // none of this feeds the gradient below, which is still computed
        // from `err` (score - blended teacher) exactly as before.
        self.target_sum += teacher as f64;
        self.target_sum_sq += (teacher as f64) * (teacher as f64);
        self.eval_teacher_sum += eval_teacher as f64;
        self.eval_teacher_sum_sq += (eval_teacher as f64) * (eval_teacher as f64);
        self.pred_eval_prod_sum += (score as f64) * (eval_teacher as f64);
        let cp_err = (score - eval_teacher) as f64;
        self.cp_component_sum += cp_err * cp_err;
        if let Some(wdl_target) = wdl_target {
            let wdl_err = (score - wdl_target) as f64;
            self.wdl_component_sum += wdl_err * wdl_err;
            self.wdl_component_count += 1;

            // `--cp-wdl-grad-trace`: two extra, diagnostic-only backward
            // passes -- one with `eval_teacher` as the sole teacher, one
            // with `wdl_target` -- decomposing the blended gradient below
            // into its two contributions. Never applied to `self.weights`;
            // see `diagnostic_backward`'s doc comment.
            if self.cp_wdl_grad_trace {
                let cp = diagnostic_backward(
                    &self.weights,
                    &l2_acc,
                    &relu_l2,
                    &acc_us,
                    &acc_them,
                    &relu_us,
                    &relu_them,
                    &active_us,
                    &active_them,
                    score - eval_teacher,
                    weight,
                );
                let wdl = diagnostic_backward(
                    &self.weights,
                    &l2_acc,
                    &relu_l2,
                    &acc_us,
                    &acc_them,
                    &relu_us,
                    &relu_them,
                    &active_us,
                    &active_them,
                    score - wdl_target,
                    weight,
                );
                for o in 0..L2 {
                    let gc = cp.d_l2_acc[o] as f64;
                    let gw = wdl.d_l2_acc[o] as f64;
                    self.l2_cp_dacc_sum[o] += gc;
                    self.l2_cp_dacc_sq_sum[o] += gc * gc;
                    self.l2_wdl_dacc_sum[o] += gw;
                    self.l2_wdl_dacc_sq_sum[o] += gw * gw;
                    self.l2_cp_wdl_dot_sum[o] += gc * gw;
                    if gc > 0.0 {
                        self.l2_cp_dacc_pos_count[o] += 1;
                    } else if gc < 0.0 {
                        self.l2_cp_dacc_neg_count[o] += 1;
                    }
                    if gw > 0.0 {
                        self.l2_wdl_dacc_pos_count[o] += 1;
                    } else if gw < 0.0 {
                        self.l2_wdl_dacc_neg_count[o] += 1;
                    }
                }
                for j in 0..L1 {
                    let gc = cp.d_ft_acc[j] as f64;
                    let gw = wdl.d_ft_acc[j] as f64;
                    self.ft_cp_dacc_sum[j] += gc;
                    self.ft_cp_dacc_sq_sum[j] += gc * gc;
                    self.ft_wdl_dacc_sum[j] += gw;
                    self.ft_wdl_dacc_sq_sum[j] += gw * gw;
                    self.ft_cp_wdl_dot_sum[j] += gc * gw;
                    if gc > 0.0 {
                        self.ft_cp_dacc_pos_count[j] += 1;
                    } else if gc < 0.0 {
                        self.ft_cp_dacc_neg_count[j] += 1;
                    }
                    if gw > 0.0 {
                        self.ft_wdl_dacc_pos_count[j] += 1;
                    } else if gw < 0.0 {
                        self.ft_wdl_dacc_neg_count[j] += 1;
                    }
                }
                self.cp_ft_grad_norm_sum += cp.ft_grad_norm;
                self.cp_ft_grad_norm_sum_sq += cp.ft_grad_norm * cp.ft_grad_norm;
                self.wdl_ft_grad_norm_sum += wdl.ft_grad_norm;
                self.wdl_ft_grad_norm_sum_sq += wdl.ft_grad_norm * wdl.ft_grad_norm;
                self.cp_l2_grad_norm_sum += cp.l2_grad_norm;
                self.cp_l2_grad_norm_sum_sq += cp.l2_grad_norm * cp.l2_grad_norm;
                self.wdl_l2_grad_norm_sum += wdl.l2_grad_norm;
                self.wdl_l2_grad_norm_sum_sq += wdl.l2_grad_norm * wdl.l2_grad_norm;
                self.cp_out_grad_norm_sum += cp.out_grad_norm;
                self.cp_out_grad_norm_sum_sq += cp.out_grad_norm * cp.out_grad_norm;
                self.wdl_out_grad_norm_sum += wdl.out_grad_norm;
                self.wdl_out_grad_norm_sum_sq += wdl.out_grad_norm * wdl.out_grad_norm;

                // Target/prediction/residual/dL-dOutput distributions --
                // explains *why* the gradient-scale fields above differ,
                // not just that they do. Scoped to this same wdl-having
                // subset (not the epoch-wide `eval_teacher_sum`/
                // `output_sum`), so every field is a fair comparison over
                // the identical position set.
                let eval_teacher_f64 = eval_teacher as f64;
                let wdl_target_f64 = wdl_target as f64;
                let score_f64 = score as f64;
                self.cp_target_sum += eval_teacher_f64;
                self.cp_target_sum_sq += eval_teacher_f64 * eval_teacher_f64;
                self.wdl_target_sum += wdl_target_f64;
                self.wdl_target_sum_sq += wdl_target_f64 * wdl_target_f64;
                self.prediction_sum += score_f64;
                self.prediction_sum_sq += score_f64 * score_f64;
                self.cp_residual_sum += cp_err;
                self.cp_residual_sum_sq += cp_err * cp_err;
                self.wdl_residual_sum += wdl_err;
                self.wdl_residual_sum_sq += wdl_err * wdl_err;
                let cp_d_output = cp.d_output as f64;
                let wdl_d_output = wdl.d_output as f64;
                self.cp_d_output_sum += cp_d_output;
                self.cp_d_output_sum_sq += cp_d_output * cp_d_output;
                self.wdl_d_output_sum += wdl_d_output;
                self.wdl_d_output_sum_sq += wdl_d_output * wdl_d_output;
            }
        }

        // ── Backward pass ─────────────────────────────────────────────────────

        let d_score = weight * 2.0 * err;
        let d_output = d_score / 64.0;

        // Output layer gradients
        let mut d_out = vec![0.0f32; L2];
        for o in 0..L2 {
            d_out[o] = d_output * relu_l2[o];
        }
        let mut d_out_bias = d_output;

        // Backprop through L2 ClippedReLU
        let mut d_l2_acc = [0.0f32; L2];
        for o in 0..L2 {
            if l2_acc[o] > 0.0 && l2_acc[o] < 127.0 {
                d_l2_acc[o] = d_output * self.weights.out[o];
            }
        }
        // `d_l2_acc[o]` is the gradient of the loss w.r.t. neuron o's own
        // pre-activation -- the per-neuron trace's most direct "which wall
        // is this neuron being pushed toward" signal (see
        // `Trainer::l2_dacc_sum`'s doc comment).
        for o in 0..L2 {
            let g = d_l2_acc[o] as f64;
            self.l2_dacc_sum[o] += g;
            self.l2_dacc_sq_sum[o] += g * g;
            if g > 0.0 {
                self.l2_dacc_pos_count[o] += 1;
            } else if g < 0.0 {
                self.l2_dacc_neg_count[o] += 1;
            }
        }

        // `--sample-grad-trace`: one record per position, up to the
        // requested limit, from the real blended `d_l2_acc` above -- never
        // reorders or otherwise changes what training does, purely reads
        // already-computed forward/backward state (see
        // `Trainer::sample_grad_trace_limit`'s doc comment).
        if self.sample_grad_trace_limit > 0 && self.l2_sample_count <= self.sample_grad_trace_limit
        {
            let cp_d_output = weight * 2.0 * (score - eval_teacher) / 64.0;
            let wdl_d_output = wdl_target.map(|t| weight * 2.0 * (score - t) / 64.0);
            let l2_grad_norm = (d_l2_acc.iter().map(|&x| (x as f64).powi(2)).sum::<f64>()).sqrt();
            let cosine_prev = self
                .sample_grad_prev_d_l2_acc
                .as_ref()
                .map(|prev| diagnostics::vector_cosine_similarity(prev, &d_l2_acc));
            let cosine_running_mean = if self.sample_grad_running_count > 0 {
                Some(diagnostics::vector_cosine_similarity(
                    &self.sample_grad_running_mean_d_l2_acc,
                    &d_l2_acc,
                ))
            } else {
                None
            };
            let l2_gate: Vec<i8> = l2_acc
                .iter()
                .map(|&x| {
                    if x <= 0.0 {
                        -1
                    } else if x >= 127.0 {
                        1
                    } else {
                        0
                    }
                })
                .collect();
            self.sample_grad_records
                .push(diagnostics::SampleGradRecord {
                    game_id,
                    game_result: format!("{game_result:?}"),
                    position_index: self.l2_sample_count,
                    prediction: score,
                    cp_target: eval_teacher,
                    wdl_target,
                    cp_d_output,
                    wdl_d_output,
                    l2_grad_vector: d_l2_acc.to_vec(),
                    l2_grad_norm,
                    cosine_prev,
                    cosine_running_mean,
                    l2_gate,
                });
            self.sample_grad_prev_d_l2_acc = Some(d_l2_acc);
            self.sample_grad_running_count += 1;
            let n = self.sample_grad_running_count as f32;
            for o in 0..L2 {
                self.sample_grad_running_mean_d_l2_acc[o] +=
                    (d_l2_acc[o] - self.sample_grad_running_mean_d_l2_acc[o]) / n;
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
        // `d_bias[j]` (the FT bias gradient) is exactly the gradient of the
        // loss w.r.t. neuron j's own pre-activation, summed across both
        // perspectives -- FT's direct counterpart to `d_l2_acc` above.
        for j in 0..L1 {
            let g = d_bias[j] as f64;
            self.ft_dacc_sum[j] += g;
            self.ft_dacc_sq_sum[j] += g * g;
            if g > 0.0 {
                self.ft_dacc_pos_count[j] += 1;
            } else if g < 0.0 {
                self.ft_dacc_neg_count[j] += 1;
            }
        }

        // ── Gradient-norm diagnostics ────────────────────────────────────────
        // Diagnostic-only, computed from the gradients above without altering
        // them. `d_ft`'s only nonzero entries are the rows touched by
        // `active_us`/`active_them`, and `d_ft[base+j] == d_acc_us[j]` (or
        // `d_acc_them[j]`) for every touched row of that side -- so its
        // squared-norm contribution is exactly `active_us.len() * Σ
        // d_acc_us[j]²` plus the `active_them` term, without a second pass
        // over the full `INPUT*L1`-length array.
        //
        // ponytail: this slightly over-counts in the (architecture-rare)
        // case where the same feature index appears in both `active_us` and
        // `active_them`, since that row's true `d_ft` value is their sum,
        // not two independent entries -- acceptable for a monitoring metric.
        let d_acc_us_sq: f64 = d_acc_us.iter().map(|&x| (x as f64).powi(2)).sum();
        let d_acc_them_sq: f64 = d_acc_them.iter().map(|&x| (x as f64).powi(2)).sum();
        let d_bias_sq: f64 = d_bias.iter().map(|&x| (x as f64).powi(2)).sum();
        let ft_grad_sq = d_acc_us_sq * active_us.len() as f64
            + d_acc_them_sq * active_them.len() as f64
            + d_bias_sq;
        let l2_grad_sq: f64 = d_l2.iter().map(|&x| (x as f64).powi(2)).sum::<f64>()
            + d_l2_bias.iter().map(|&x| (x as f64).powi(2)).sum::<f64>();
        let out_grad_sq: f64 =
            d_out.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() + (d_out_bias as f64).powi(2);

        let ft_grad_norm = ft_grad_sq.sqrt();
        let l2_grad_norm = l2_grad_sq.sqrt();
        let out_grad_norm = out_grad_sq.sqrt();
        self.ft_grad_norm_sum += ft_grad_norm;
        self.ft_grad_norm_sum_sq += ft_grad_norm * ft_grad_norm;
        self.l2_grad_norm_sum += l2_grad_norm;
        self.l2_grad_norm_sum_sq += l2_grad_norm * l2_grad_norm;
        self.out_grad_norm_sum += out_grad_norm;
        self.out_grad_norm_sum_sq += out_grad_norm * out_grad_norm;
        self.out_grad_norm_values.push(out_grad_norm as f32);
        let global_grad_norm = (ft_grad_sq + l2_grad_sq + out_grad_sq).sqrt();
        self.global_grad_norm_values.push(global_grad_norm as f32);

        // ── Per-layer gradient clipping (optional) ───────────────────────────
        // Each layer's gradient is compared against *its own* norm and *its
        // own* threshold, independent of the other layers -- unlike the
        // global-norm clipping below, setting only `out_clip_norm` leaves
        // FT/L2 completely untouched (a real single-variable change). Applied
        // before the diagnostics-vs-clip ordering matters the same way as
        // global clipping: the sums/percentiles above already captured the
        // unclipped norms, so this can't retroactively change what a
        // threshold-selection read from this run's own output.
        if let Some(clip_norm) = self.ft_clip_norm {
            let clip_norm = clip_norm as f64;
            if ft_grad_norm > clip_norm {
                self.ft_clip_count += 1;
                let scale = (clip_norm / ft_grad_norm) as f32;
                d_ft.iter_mut().for_each(|x| *x *= scale);
                d_bias.iter_mut().for_each(|x| *x *= scale);
            }
        }
        if let Some(clip_norm) = self.l2_clip_norm {
            let clip_norm = clip_norm as f64;
            if l2_grad_norm > clip_norm {
                self.l2_clip_count += 1;
                let scale = (clip_norm / l2_grad_norm) as f32;
                d_l2.iter_mut().for_each(|x| *x *= scale);
                d_l2_bias.iter_mut().for_each(|x| *x *= scale);
            }
        }
        let mut out_grad_norm_after = out_grad_norm;
        if let Some(clip_norm) = self.out_clip_norm {
            let clip_norm = clip_norm as f64;
            if out_grad_norm > clip_norm {
                self.out_clip_count += 1;
                let scale = (clip_norm / out_grad_norm) as f32;
                d_out.iter_mut().for_each(|x| *x *= scale);
                d_out_bias *= scale;
                out_grad_norm_after = clip_norm;
            }
        }
        self.out_grad_norm_after_sum += out_grad_norm_after;
        self.out_grad_norm_after_sum_sq += out_grad_norm_after * out_grad_norm_after;

        // ── Global gradient clipping (optional) ──────────────────────────────
        // Global-norm clipping: if the whole-network gradient norm exceeds
        // `grad_clip_norm`, scale every layer's gradient down by the same
        // factor (direction preserved, only magnitude reduced). Applied
        // after the diagnostics above capture the unclipped norm, so
        // `global_grad_norm_p95`/`p99` always describe the natural
        // distribution a threshold should be chosen from, not a value
        // that's already been clamped by whatever threshold is active.
        // Independent of the per-layer clipping above -- if both are set,
        // this acts on whatever the per-layer step already produced (an
        // untested combination; the 2026-07 experiments use exactly one
        // clipping mechanism at a time).
        if let Some(clip_norm) = self.grad_clip_norm {
            let clip_norm = clip_norm as f64;
            if global_grad_norm > clip_norm {
                self.grad_clip_count += 1;
                let scale = (clip_norm / global_grad_norm) as f32;
                d_ft.iter_mut().for_each(|x| *x *= scale);
                d_bias.iter_mut().for_each(|x| *x *= scale);
                d_l2.iter_mut().for_each(|x| *x *= scale);
                d_l2_bias.iter_mut().for_each(|x| *x *= scale);
                d_out.iter_mut().for_each(|x| *x *= scale);
                d_out_bias *= scale;
            }
        }

        // ── Adam update ───────────────────────────────────────────────────────

        self.weights.step += 1;
        let t = self.weights.step;
        let lr = self.lr;

        // Freeze gates (diagnostic only, `--diagnostic-freeze-layer`). A
        // frozen layer's params *and* Adam `m`/`v` are left completely
        // untouched this position -- the gradients above (`d_ft`/`d_l2`/
        // `d_out` etc.) were already fully computed through this layer's
        // *current* (frozen) weight values by the ordinary backward pass,
        // so this only discards that layer's own parameter update; it does
        // not cut the gradient path to upstream/downstream layers (see
        // `diagnostic_freeze_layer`'s doc comment -- this is deliberately
        // not stop-gradient).
        let freeze_active = self.diagnostic_freeze_layer.is_some()
            && self.l2_sample_count >= self.diagnostic_freeze_from_position
            && self.l2_sample_count <= self.diagnostic_freeze_until_position;
        let ft_frozen = freeze_active && self.diagnostic_freeze_layer == Some(FreezeLayer::Ft);
        let l2_frozen = freeze_active && self.diagnostic_freeze_layer == Some(FreezeLayer::L2);
        let out_frozen = freeze_active && self.diagnostic_freeze_layer == Some(FreezeLayer::Out);

        let (ft_update_sq, ft_bias_update_sq) = if ft_frozen {
            d_bias.iter_mut().for_each(|x| *x = 0.0);
            (0.0, 0.0)
        } else {
            let ft_update_sq = adam_update_slice(
                &mut self.weights.ft,
                &mut self.weights.ft_m,
                &mut self.weights.ft_v,
                &mut d_ft,
                lr,
                t,
            );
            let ft_bias_update_sq = adam_update_slice(
                &mut self.weights.ft_bias,
                &mut self.weights.bias_m,
                &mut self.weights.bias_v,
                &mut d_bias,
                lr,
                t,
            );
            (ft_update_sq, ft_bias_update_sq)
        };
        // `d_bias` now holds each FT neuron's own applied bias delta (see
        // `adam_update_slice`'s doc comment) -- exactly the per-neuron
        // trace's update-norm signal (`Trainer::ft_bias_update_sq_sum`).
        // Zeroed above when frozen, so this correctly records "no update
        // applied" rather than the discarded raw gradient.
        for j in 0..L1 {
            self.ft_bias_update_sq_sum[j] += (d_bias[j] as f64).powi(2);
        }
        let (l2_update_sq, l2_bias_update_sq) = if l2_frozen {
            d_l2_bias.iter_mut().for_each(|x| *x = 0.0);
            (0.0, 0.0)
        } else {
            let l2_update_sq = adam_update_slice(
                &mut self.weights.l2,
                &mut self.weights.l2_m,
                &mut self.weights.l2_v,
                &mut d_l2,
                lr,
                t,
            );
            let l2_bias_update_sq = adam_update_slice(
                &mut self.weights.l2_bias,
                &mut self.weights.l2bias_m,
                &mut self.weights.l2bias_v,
                &mut d_l2_bias,
                lr,
                t,
            );
            (l2_update_sq, l2_bias_update_sq)
        };
        for o in 0..L2 {
            self.l2_bias_update_sq_sum[o] += (d_l2_bias[o] as f64).powi(2);
        }
        let (out_update_sq, out_bias_delta) = if out_frozen {
            (0.0, 0.0f32)
        } else {
            let out_update_sq = adam_update_slice(
                &mut self.weights.out,
                &mut self.weights.out_m,
                &mut self.weights.out_v,
                &mut d_out,
                lr,
                t,
            );
            let out_bias_delta = adam_update_scalar(
                &mut self.weights.out_bias,
                &mut self.weights.obias_m,
                &mut self.weights.obias_v,
                d_out_bias,
                lr,
                t,
            );
            (out_update_sq, out_bias_delta)
        };

        // Diagnostic-only: the applied update norm per layer, as opposed to
        // the gradient norm captured above -- see the `Trainer` field docs
        // for why these can diverge under Adam.
        let ft_update_norm = (ft_update_sq + ft_bias_update_sq).sqrt();
        let l2_update_norm = (l2_update_sq + l2_bias_update_sq).sqrt();
        let out_update_norm = (out_update_sq + (out_bias_delta as f64).powi(2)).sqrt();
        self.ft_update_norm_sum += ft_update_norm;
        self.ft_update_norm_sum_sq += ft_update_norm * ft_update_norm;
        self.l2_update_norm_sum += l2_update_norm;
        self.l2_update_norm_sum_sq += l2_update_norm * l2_update_norm;
        self.out_update_norm_sum += out_update_norm;
        self.out_update_norm_sum_sq += out_update_norm * out_update_norm;

        self.maybe_trace_snapshot();
    }

    /// If `l2_sample_count` (positions fully processed so far this epoch)
    /// matches a requested `--trace-positions` point, builds and records a
    /// `TraceSnapshot` from the accumulators above. No-op (one `HashSet`
    /// lookup) when `trace_positions` is empty, i.e. the flag was omitted.
    fn maybe_trace_snapshot(&mut self) {
        if !self.trace_positions.contains(&self.l2_sample_count) {
            return;
        }
        if self.weight_snapshot_trace {
            self.weight_snapshots
                .push((self.l2_sample_count, self.weights.clone()));
        }
        let l2_weight_row_norm: Vec<f32> = (0..L2)
            .map(|o| {
                (0..2 * L1)
                    .map(|j| self.weights.l2[j * L2 + o].powi(2))
                    .sum::<f32>()
                    .sqrt()
            })
            .collect();
        let ft_weight_row_norm: Vec<f32> = (0..L1)
            .map(|j| {
                (0..INPUT)
                    .map(|feat| self.weights.ft[feat * L1 + j].powi(2))
                    .sum::<f32>()
                    .sqrt()
            })
            .collect();
        let l2 = diagnostics::build_trace_layer_snapshot(
            &self.l2_values,
            &self.l2_weighted_input_values,
            &self.l2_zero_count,
            &self.l2_sat_count,
            self.l2_sample_count,
            l2_weight_row_norm,
            self.weights.l2_bias.clone(),
            &self.l2_dacc_sum,
            &self.l2_dacc_sq_sum,
            &self.l2_dacc_pos_count,
            &self.l2_dacc_neg_count,
            &self.l2_bias_update_sq_sum,
        );
        let ft = diagnostics::build_trace_layer_snapshot(
            &[], // FT's own pre-activation history isn't accumulated per-sample
            &[], // (no weighted-input split for FT either -- see the doc comment)
            &self.ft_zero_count,
            &self.ft_sat_count,
            self.l2_sample_count,
            ft_weight_row_norm,
            self.weights.ft_bias.clone(),
            &self.ft_dacc_sum,
            &self.ft_dacc_sq_sum,
            &self.ft_dacc_pos_count,
            &self.ft_dacc_neg_count,
            &self.ft_bias_update_sq_sum,
        );
        let (l2_input_norm_mean, l2_input_norm_std) = diagnostics::mean_std(
            self.l2_input_norm_sum,
            self.l2_input_norm_sq_sum,
            self.l2_sample_count,
        );
        let (ft_output_mean, ft_output_std) = diagnostics::mean_std(
            self.ft_output_sum,
            self.ft_output_sum_sq,
            self.ft_output_count,
        );
        // `wdl_component_count` is exactly "positions this diagnostic ran
        // on" (same gating `--cp-wdl-grad-trace`'s hook in `train_position`
        // uses) -- `None` when the flag never ran (off, or a run with no
        // WDL signal at all), not an empty/zeroed struct.
        let cp_wdl = if self.cp_wdl_grad_trace && self.wdl_component_count > 0 {
            let n = self.wdl_component_count;
            let (cp_ft_grad_rms, _) =
                diagnostics::mean_std(self.cp_ft_grad_norm_sum, self.cp_ft_grad_norm_sum_sq, n);
            let (wdl_ft_grad_rms, _) =
                diagnostics::mean_std(self.wdl_ft_grad_norm_sum, self.wdl_ft_grad_norm_sum_sq, n);
            let (cp_l2_grad_rms, _) =
                diagnostics::mean_std(self.cp_l2_grad_norm_sum, self.cp_l2_grad_norm_sum_sq, n);
            let (wdl_l2_grad_rms, _) =
                diagnostics::mean_std(self.wdl_l2_grad_norm_sum, self.wdl_l2_grad_norm_sum_sq, n);
            let (cp_out_grad_rms, _) =
                diagnostics::mean_std(self.cp_out_grad_norm_sum, self.cp_out_grad_norm_sum_sq, n);
            let (wdl_out_grad_rms, _) =
                diagnostics::mean_std(self.wdl_out_grad_norm_sum, self.wdl_out_grad_norm_sum_sq, n);
            let (cp_target_mean, cp_target_std) =
                diagnostics::mean_std(self.cp_target_sum, self.cp_target_sum_sq, n);
            let (wdl_target_mean, wdl_target_std) =
                diagnostics::mean_std(self.wdl_target_sum, self.wdl_target_sum_sq, n);
            let (prediction_mean, prediction_std) =
                diagnostics::mean_std(self.prediction_sum, self.prediction_sum_sq, n);
            let (cp_residual_mean, cp_residual_std) =
                diagnostics::mean_std(self.cp_residual_sum, self.cp_residual_sum_sq, n);
            let (wdl_residual_mean, wdl_residual_std) =
                diagnostics::mean_std(self.wdl_residual_sum, self.wdl_residual_sum_sq, n);
            let (cp_d_output_mean, cp_d_output_std) =
                diagnostics::mean_std(self.cp_d_output_sum, self.cp_d_output_sum_sq, n);
            let (wdl_d_output_mean, wdl_d_output_std) =
                diagnostics::mean_std(self.wdl_d_output_sum, self.wdl_d_output_sum_sq, n);
            Some(diagnostics::CpWdlTrace {
                l2: diagnostics::build_cp_wdl_layer_trace(
                    &self.l2_cp_dacc_sum,
                    &self.l2_cp_dacc_sq_sum,
                    &self.l2_cp_dacc_pos_count,
                    &self.l2_cp_dacc_neg_count,
                    &self.l2_wdl_dacc_sum,
                    &self.l2_wdl_dacc_sq_sum,
                    &self.l2_wdl_dacc_pos_count,
                    &self.l2_wdl_dacc_neg_count,
                    &self.l2_cp_wdl_dot_sum,
                    n,
                ),
                ft: diagnostics::build_cp_wdl_layer_trace(
                    &self.ft_cp_dacc_sum,
                    &self.ft_cp_dacc_sq_sum,
                    &self.ft_cp_dacc_pos_count,
                    &self.ft_cp_dacc_neg_count,
                    &self.ft_wdl_dacc_sum,
                    &self.ft_wdl_dacc_sq_sum,
                    &self.ft_wdl_dacc_pos_count,
                    &self.ft_wdl_dacc_neg_count,
                    &self.ft_cp_wdl_dot_sum,
                    n,
                ),
                cp_ft_grad_rms,
                wdl_ft_grad_rms,
                cp_l2_grad_rms,
                wdl_l2_grad_rms,
                cp_out_grad_rms,
                wdl_out_grad_rms,
                cp_target_mean,
                cp_target_std,
                wdl_target_mean,
                wdl_target_std,
                prediction_mean,
                prediction_std,
                cp_residual_mean,
                cp_residual_std,
                wdl_residual_mean,
                wdl_residual_std,
                cp_d_output_mean,
                cp_d_output_std,
                wdl_d_output_mean,
                wdl_d_output_std,
            })
        } else {
            None
        };
        self.trace_snapshots.push(diagnostics::TraceSnapshot {
            position_index: self.l2_sample_count,
            l2,
            ft,
            l2_input_norm_mean,
            l2_input_norm_std,
            ft_output_mean,
            ft_output_std,
            cp_wdl,
        });
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
        self.l2_zero_count.iter_mut().for_each(|c| *c = 0);
        self.l2_sat_count.iter_mut().for_each(|c| *c = 0);
        self.l2_sample_count = 0;
        self.l2_values.iter_mut().for_each(|v| v.clear());
        self.ft_grad_norm_sum = 0.0;
        self.ft_grad_norm_sum_sq = 0.0;
        self.l2_grad_norm_sum = 0.0;
        self.l2_grad_norm_sum_sq = 0.0;
        self.out_grad_norm_sum = 0.0;
        self.out_grad_norm_sum_sq = 0.0;
        self.global_grad_norm_values.clear();
        self.ft_update_norm_sum = 0.0;
        self.ft_update_norm_sum_sq = 0.0;
        self.l2_update_norm_sum = 0.0;
        self.l2_update_norm_sum_sq = 0.0;
        self.out_update_norm_sum = 0.0;
        self.out_update_norm_sum_sq = 0.0;
        self.target_sum = 0.0;
        self.target_sum_sq = 0.0;
        self.eval_teacher_sum = 0.0;
        self.eval_teacher_sum_sq = 0.0;
        self.pred_eval_prod_sum = 0.0;
        self.cp_component_sum = 0.0;
        self.wdl_component_sum = 0.0;
        self.wdl_component_count = 0;
        self.grad_clip_count = 0;
        self.ft_clip_count = 0;
        self.l2_clip_count = 0;
        self.out_clip_count = 0;
        self.out_grad_norm_values.clear();
        self.out_grad_norm_after_sum = 0.0;
        self.out_grad_norm_after_sum_sq = 0.0;
        self.cache_hits = 0;
        self.cache_misses = 0;
        self.trace_snapshots.clear();
        self.weight_snapshots.clear();
        self.l2_weighted_input_values
            .iter_mut()
            .for_each(|v| v.clear());
        self.l2_dacc_sum.iter_mut().for_each(|x| *x = 0.0);
        self.l2_dacc_sq_sum.iter_mut().for_each(|x| *x = 0.0);
        self.l2_dacc_pos_count.iter_mut().for_each(|x| *x = 0);
        self.l2_dacc_neg_count.iter_mut().for_each(|x| *x = 0);
        self.ft_dacc_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_dacc_sq_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_dacc_pos_count.iter_mut().for_each(|x| *x = 0);
        self.ft_dacc_neg_count.iter_mut().for_each(|x| *x = 0);
        self.l2_bias_update_sq_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_bias_update_sq_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_zero_count.iter_mut().for_each(|x| *x = 0);
        self.ft_sat_count.iter_mut().for_each(|x| *x = 0);
        self.l2_input_norm_sum = 0.0;
        self.l2_input_norm_sq_sum = 0.0;
        self.ft_output_sum = 0.0;
        self.ft_output_sum_sq = 0.0;
        self.ft_output_count = 0;
        self.l2_cp_dacc_sum.iter_mut().for_each(|x| *x = 0.0);
        self.l2_cp_dacc_sq_sum.iter_mut().for_each(|x| *x = 0.0);
        self.l2_cp_dacc_pos_count.iter_mut().for_each(|x| *x = 0);
        self.l2_cp_dacc_neg_count.iter_mut().for_each(|x| *x = 0);
        self.l2_wdl_dacc_sum.iter_mut().for_each(|x| *x = 0.0);
        self.l2_wdl_dacc_sq_sum.iter_mut().for_each(|x| *x = 0.0);
        self.l2_wdl_dacc_pos_count.iter_mut().for_each(|x| *x = 0);
        self.l2_wdl_dacc_neg_count.iter_mut().for_each(|x| *x = 0);
        self.l2_cp_wdl_dot_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_cp_dacc_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_cp_dacc_sq_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_cp_dacc_pos_count.iter_mut().for_each(|x| *x = 0);
        self.ft_cp_dacc_neg_count.iter_mut().for_each(|x| *x = 0);
        self.ft_wdl_dacc_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_wdl_dacc_sq_sum.iter_mut().for_each(|x| *x = 0.0);
        self.ft_wdl_dacc_pos_count.iter_mut().for_each(|x| *x = 0);
        self.ft_wdl_dacc_neg_count.iter_mut().for_each(|x| *x = 0);
        self.ft_cp_wdl_dot_sum.iter_mut().for_each(|x| *x = 0.0);
        self.cp_ft_grad_norm_sum = 0.0;
        self.cp_ft_grad_norm_sum_sq = 0.0;
        self.wdl_ft_grad_norm_sum = 0.0;
        self.wdl_ft_grad_norm_sum_sq = 0.0;
        self.cp_l2_grad_norm_sum = 0.0;
        self.cp_l2_grad_norm_sum_sq = 0.0;
        self.wdl_l2_grad_norm_sum = 0.0;
        self.wdl_l2_grad_norm_sum_sq = 0.0;
        self.cp_out_grad_norm_sum = 0.0;
        self.cp_out_grad_norm_sum_sq = 0.0;
        self.wdl_out_grad_norm_sum = 0.0;
        self.wdl_out_grad_norm_sum_sq = 0.0;
        self.cp_target_sum = 0.0;
        self.cp_target_sum_sq = 0.0;
        self.wdl_target_sum = 0.0;
        self.wdl_target_sum_sq = 0.0;
        self.prediction_sum = 0.0;
        self.prediction_sum_sq = 0.0;
        self.cp_residual_sum = 0.0;
        self.cp_residual_sum_sq = 0.0;
        self.wdl_residual_sum = 0.0;
        self.wdl_residual_sum_sq = 0.0;
        self.cp_d_output_sum = 0.0;
        self.cp_d_output_sum_sq = 0.0;
        self.wdl_d_output_sum = 0.0;
        self.wdl_d_output_sum_sq = 0.0;
        self.sample_grad_records.clear();
        self.sample_grad_prev_d_l2_acc = None;
        self.sample_grad_running_mean_d_l2_acc = [0.0; L2];
        self.sample_grad_running_count = 0;
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

/// Returns the sum of squared per-parameter deltas actually applied --
/// Result of `diagnostic_backward`: the per-neuron and whole-layer
/// gradients L2/FT/output *would* have if `err` were the sole error term.
struct DiagnosticGrad {
    /// Per-neuron L2 gradient (`d_l2_acc[o]` in `train_position`'s own
    /// backward pass) -- the direct "which wall is neuron o being pushed
    /// toward" signal, for this `err` alone.
    d_l2_acc: [f32; L2],
    /// Per-neuron FT gradient (`d_bias[j]`'s counterpart), summed across
    /// both perspectives, for this `err` alone.
    d_ft_acc: Vec<f32>,
    l2_grad_norm: f64,
    ft_grad_norm: f64,
    out_grad_norm: f64,
    /// `d_output` (gradient of the loss w.r.t. the network's scalar
    /// output) for this `err` alone -- the quantity everything else in
    /// this struct backpropagates from.
    d_output: f32,
}

/// Diagnostic-only: recomputes the backward pass `train_position` already
/// ran, but for a single-term `err` (CP-only or WDL-only) instead of the
/// blended one -- decomposing `--cp-wdl-grad-trace`'s gradient into its two
/// contributions. Structurally identical to `train_position`'s own
/// backward pass (same formulas, same shortcuts, e.g. `ft_grad_sq`'s
/// active-feature-count trick), deliberately not refactored to share code
/// with it, so this is trivially auditable against the tested main path
/// rather than introducing a new derivation to trust. Reads `w` and the
/// forward-pass state already computed for this position; never touches
/// `self.weights`, `self.weights.step`, or any Adam moment -- purely a
/// side computation, discarded after its stats are accumulated.
#[allow(clippy::too_many_arguments)]
fn diagnostic_backward(
    w: &TrainWeights,
    l2_acc: &[f32],
    relu_l2: &[f32],
    acc_us: &[f32],
    acc_them: &[f32],
    relu_us: &[f32],
    relu_them: &[f32],
    active_us: &[usize],
    active_them: &[usize],
    err: f32,
    weight: f32,
) -> DiagnosticGrad {
    let d_score = weight * 2.0 * err;
    let d_output = d_score / 64.0;

    let d_out: Vec<f32> = relu_l2.iter().map(|&r| d_output * r).collect();
    let d_out_bias = d_output;

    let mut d_l2_acc = [0.0f32; L2];
    for o in 0..L2 {
        if l2_acc[o] > 0.0 && l2_acc[o] < 127.0 {
            d_l2_acc[o] = d_output * w.out[o];
        }
    }

    let mut d_l2 = vec![0.0f32; 2 * L1 * L2];
    let mut d_l2_bias = [0.0f32; L2];
    let mut d_relu_us = vec![0.0f32; L1];
    let mut d_relu_them = vec![0.0f32; L1];
    for j in 0..L1 {
        let base_us = j * L2;
        let base_them = (L1 + j) * L2;
        for o in 0..L2 {
            let g = d_l2_acc[o];
            d_l2[base_us + o] += g * relu_us[j];
            d_l2[base_them + o] += g * relu_them[j];
            d_relu_us[j] += g * w.l2[base_us + o];
            d_relu_them[j] += g * w.l2[base_them + o];
        }
    }
    d_l2_bias[..L2].copy_from_slice(&d_l2_acc[..L2]);

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
    let mut d_ft_acc = vec![0.0f32; L1];
    for j in 0..L1 {
        d_ft_acc[j] = d_acc_us[j] + d_acc_them[j];
    }

    // Same shortcut `train_position`'s own `ft_grad_sq` uses (see its
    // comment) -- avoids a second materialization of the full
    // `INPUT*L1`-length FT weight-gradient array.
    let d_acc_us_sq: f64 = d_acc_us.iter().map(|&x| (x as f64).powi(2)).sum();
    let d_acc_them_sq: f64 = d_acc_them.iter().map(|&x| (x as f64).powi(2)).sum();
    let d_bias_sq: f64 = d_ft_acc.iter().map(|&x| (x as f64).powi(2)).sum();
    let ft_grad_sq =
        d_acc_us_sq * active_us.len() as f64 + d_acc_them_sq * active_them.len() as f64 + d_bias_sq;
    let l2_grad_sq: f64 = d_l2.iter().map(|&x| (x as f64).powi(2)).sum::<f64>()
        + d_l2_bias.iter().map(|&x| (x as f64).powi(2)).sum::<f64>();
    let out_grad_sq: f64 =
        d_out.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() + (d_out_bias as f64).powi(2);

    DiagnosticGrad {
        d_l2_acc,
        d_ft_acc,
        l2_grad_norm: l2_grad_sq.sqrt(),
        ft_grad_norm: ft_grad_sq.sqrt(),
        out_grad_norm: out_grad_sq.sqrt(),
        d_output,
    }
}

/// Adam's moment decay means every parameter in `params` gets a (possibly
/// tiny) nonzero update even where `grads[i] == 0`, so this is the true
/// applied-update norm for the slice, not just an approximation over the
/// nonzero-gradient subset. Used for per-layer update-norm diagnostics
/// (distinct from *gradient* norm -- Adam's √v̂ normalization means a
/// smaller gradient doesn't necessarily mean a smaller applied step).
/// `grads` is overwritten in place with each element's applied delta (zero
/// extra allocation) -- callers that need a per-element breakdown (the
/// `--trace-positions` per-neuron update norm; see `Trainer::l2_bias_update_sq_sum`)
/// read it back after the call instead of only getting the whole-slice
/// `delta_sq_sum` this still returns. Callers that don't need per-element
/// deltas just let the (about to be dropped) buffer be repurposed, same as
/// today.
fn adam_update_slice(
    params: &mut [f32],
    m: &mut [f32],
    v: &mut [f32],
    grads: &mut [f32],
    lr: f32,
    t: u64,
) -> f64 {
    let mut delta_sq_sum = 0.0f64;
    for i in 0..params.len() {
        let delta = adam_update_scalar(&mut params[i], &mut m[i], &mut v[i], grads[i], lr, t);
        delta_sq_sum += (delta as f64) * (delta as f64);
        grads[i] = delta;
    }
    delta_sq_sum
}

#[inline]
fn adam_update_scalar(
    param: &mut f32,
    m: &mut f32,
    v: &mut f32,
    grad: f32,
    lr: f32,
    t: u64,
) -> f32 {
    const B1: f32 = 0.9;
    const B2: f32 = 0.999;
    const EPS: f32 = 1e-8;

    *m = B1 * *m + (1.0 - B1) * grad;
    *v = B2 * *v + (1.0 - B2) * grad * grad;

    let m_hat = *m / (1.0 - B1.powi(t as i32));
    let v_hat = *v / (1.0 - B2.powi(t as i32));

    let delta = -lr * m_hat / (v_hat.sqrt() + EPS);
    *param += delta;
    delta
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
        let w = TrainWeights::new_seeded(42, 0.5);
        // Any single FT row (one input feature's L1 contributions) must not
        // collapse to a single repeated scalar -- that's the exact failure
        // this init replaces (see `new_seeded`'s doc comment).
        assert!(variance(&w.ft[0..L1]) > 0.0);
        assert!(variance(&w.l2[0..L2]) > 0.0);
        assert!(variance(&w.out) > 0.0);
    }

    #[test]
    fn seeded_init_is_deterministic() {
        let a = TrainWeights::new_seeded(42, 0.5);
        let b = TrainWeights::new_seeded(42, 0.5);
        assert_eq!(a.ft, b.ft);
        assert_eq!(a.l2, b.l2);
        assert_eq!(a.out, b.out);
    }

    #[test]
    fn l2_bias_init_only_touches_l2_bias() {
        // l2_bias is a constant fill, not RNG-drawn -- changing it must not
        // perturb the RNG stream that produces ft/l2/out, since a shifted
        // stream would silently confound any experiment that varies
        // l2_bias_init while trying to hold the rest of init fixed.
        let default_bias = TrainWeights::new_seeded(42, 0.5);
        let custom_bias = TrainWeights::new_seeded(42, 3.0);
        assert_eq!(custom_bias.l2_bias, vec![3.0; L2]);
        assert_eq!(default_bias.l2_bias, vec![0.5; L2]);
        assert_eq!(default_bias.ft, custom_bias.ft);
        assert_eq!(default_bias.l2, custom_bias.l2);
        assert_eq!(default_bias.out, custom_bias.out);
        assert_eq!(default_bias.ft_bias, custom_bias.ft_bias);
    }

    #[test]
    fn from_nnue_weights_round_trips_forward_output() {
        // to_nnue_weights (quantise) then from_nnue_weights (dequantise)
        // should reproduce the same forward-pass score, up to i16
        // quantisation rounding -- this is what `--eval-only` relies on to
        // score an already-trained checkpoint the same way training did.
        let mut t = Trainer::new(42, 0.5);
        let board = Board::startpos();
        let before = t.forward(&board);
        let nn = t.weights.to_nnue_weights();
        t.weights = TrainWeights::from_nnue_weights(&nn);
        let after = t.forward(&board);
        assert!(
            (before - after).abs() < 1.0,
            "before={before} after={after}"
        );
    }

    #[test]
    fn seeded_init_differs_across_seeds() {
        let a = TrainWeights::new_seeded(1, 0.5);
        let b = TrainWeights::new_seeded(2, 0.5);
        assert_ne!(a.ft, b.ft);
    }

    #[test]
    fn wdl_target_black_win_from_black_perspective_is_max() {
        assert_eq!(
            wdl_target_cp(GameResult::BlackWin, Color::Black, 1200.0),
            Some(600.0)
        );
    }

    #[test]
    fn wdl_target_black_win_from_white_perspective_is_min() {
        assert_eq!(
            wdl_target_cp(GameResult::BlackWin, Color::White, 1200.0),
            Some(-600.0)
        );
    }

    #[test]
    fn wdl_target_white_win_from_white_perspective_is_max() {
        assert_eq!(
            wdl_target_cp(GameResult::WhiteWin, Color::White, 1200.0),
            Some(600.0)
        );
    }

    #[test]
    fn wdl_target_white_win_from_black_perspective_is_min() {
        assert_eq!(
            wdl_target_cp(GameResult::WhiteWin, Color::Black, 1200.0),
            Some(-600.0)
        );
    }

    #[test]
    fn wdl_target_draw_is_zero_regardless_of_perspective() {
        assert_eq!(
            wdl_target_cp(GameResult::Draw, Color::Black, 1200.0),
            Some(0.0)
        );
        assert_eq!(
            wdl_target_cp(GameResult::Draw, Color::White, 1200.0),
            Some(0.0)
        );
    }

    #[test]
    fn wdl_target_unknown_result_has_no_signal() {
        assert_eq!(
            wdl_target_cp(GameResult::Unknown, Color::Black, 1200.0),
            None
        );
        assert_eq!(
            wdl_target_cp(GameResult::Unknown, Color::White, 1200.0),
            None
        );
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
    fn compute_lr_short_run_reproduces_epoch3_of_the_real_20_epoch_schedule() {
        // This is the 2026-07 schedule-horizon bug, pinned down as a numeric
        // regression test: `--epochs 3` used to pass `total_epochs=3`,
        // compressing the entire cosine decay into 3 epochs and landing
        // epoch 3 at the min_lr floor (0.00001) instead of the correct,
        // barely-decayed value from the real 20-epoch B/C schedule. Callers
        // must pass the schedule horizon (20), not the run length (3).
        let lr = compute_lr(LrSchedule::Cosine, 0.001, 0.00001, 3, 20, 1);
        assert!(
            (lr - 0.000992).abs() < 1e-6,
            "epoch3 lr={lr}, expected ~0.000992 (not the min_lr floor 0.00001)"
        );
    }

    #[test]
    fn compute_lr_first_3_epochs_of_20_match_hand_computed_prefix() {
        // The "prefix-match" property `--lr-schedule-epochs` relies on: a
        // 3-epoch run and the real 20-epoch B/C schedule must agree
        // epoch-for-epoch wherever they overlap. `compute_lr` never receives
        // "how many epochs will actually run" -- only `total_epochs` (the
        // schedule horizon) -- so as long as callers pass total_epochs=20
        // regardless of run length, this holds by construction; pin the
        // expected sequence down numerically so a future signature change
        // can't quietly break it.
        let expected = [0.001, 0.001, 0.000992];
        for (i, want) in expected.iter().enumerate() {
            let epoch = (i + 1) as u32;
            let got = compute_lr(LrSchedule::Cosine, 0.001, 0.00001, epoch, 20, 1);
            assert!(
                (got - want).abs() < 1e-6,
                "epoch {epoch}: got {got}, want {want}"
            );
        }
    }

    #[test]
    fn resolve_schedule_epochs_defaults_to_epochs_when_omitted() {
        // Reproduces today's (pre-flag) behavior exactly when the new flag
        // is not passed.
        assert_eq!(resolve_schedule_epochs(3, None, 1).unwrap(), 3);
        assert_eq!(resolve_schedule_epochs(20, None, 0).unwrap(), 20);
    }

    #[test]
    fn resolve_schedule_epochs_accepts_a_longer_explicit_horizon() {
        assert_eq!(resolve_schedule_epochs(3, Some(20), 1).unwrap(), 20);
    }

    #[test]
    fn resolve_schedule_epochs_rejects_zero() {
        assert!(resolve_schedule_epochs(3, Some(0), 0).is_err());
    }

    #[test]
    fn resolve_schedule_epochs_rejects_warmup_exceeding_schedule_epochs() {
        assert!(resolve_schedule_epochs(3, Some(5), 6).is_err());
    }

    #[test]
    fn resolve_schedule_epochs_rejects_schedule_epochs_less_than_epochs() {
        // Must error, not silently clamp -- an implicit floor would hide
        // exactly the mistake that caused the 2026-07 schedule bug.
        assert!(resolve_schedule_epochs(20, Some(3), 0).is_err());
    }

    #[test]
    fn resolve_schedule_epochs_epochs_zero_never_errors() {
        // `--epochs 0` (dumping an untrained checkpoint) must keep working
        // unvalidated -- the epoch loop never runs, so no schedule value,
        // including the default-to-0 case, is ever actually wrong.
        assert_eq!(resolve_schedule_epochs(0, None, 0).unwrap(), 0);
        assert_eq!(resolve_schedule_epochs(0, None, 5).unwrap(), 0);
        assert_eq!(resolve_schedule_epochs(0, Some(20), 0).unwrap(), 20);
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
        let mut trainer = Trainer::new(1, 0.5);
        let mut cache: HashMap<String, i32> = HashMap::new();
        let mut board = Board::startpos();

        let (first, _) = trainer.position_teacher_components(
            &mut board,
            GameResult::Unknown,
            2,
            &mut cache,
            1200.0,
        );
        assert_eq!(trainer.cache_misses, 1);
        assert_eq!(trainer.cache_hits, 0);
        assert_eq!(cache.len(), 1);

        let mut board_again = Board::startpos();
        let (second, _) = trainer.position_teacher_components(
            &mut board_again,
            GameResult::Unknown,
            2,
            &mut cache,
            1200.0,
        );
        assert_eq!(trainer.cache_misses, 1, "second call must not re-search");
        assert_eq!(trainer.cache_hits, 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(first, second);
    }

    #[test]
    fn train_position_grad_clip_norm_shrinks_the_applied_update() {
        let board = Board::startpos();

        // A large teacher error (-600 vs. a near-zero fresh-init prediction)
        // to force a real, non-tiny gradient.
        let mut unclipped = Trainer::new(1, 0.5);
        unclipped.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        let mut clipped = Trainer::new(1, 0.5);
        clipped.grad_clip_norm = Some(1.0); // far below any real gradient norm
        clipped.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        assert_eq!(
            clipped.grad_clip_count, 1,
            "the tiny threshold must trigger"
        );
        assert_eq!(unclipped.grad_clip_count, 0);
        // Both start from the identical seeded init, so a smaller applied
        // update norm directly reflects the clip, not initialization noise.
        assert!(
            clipped.ft_update_norm_sum < unclipped.ft_update_norm_sum,
            "clipped={} unclipped={}",
            clipped.ft_update_norm_sum,
            unclipped.ft_update_norm_sum
        );
        // The unclipped diagnostic still records the *natural* (unclipped)
        // gradient norm -- clipping must not retroactively shrink what the
        // percentile diagnostics report, or a clip threshold could never be
        // chosen from a run's own output.
        assert_eq!(
            clipped.global_grad_norm_values[0],
            unclipped.global_grad_norm_values[0]
        );
    }

    #[test]
    fn train_position_out_clip_norm_leaves_ft_and_l2_untouched() {
        let board = Board::startpos();

        let mut unclipped = Trainer::new(1, 0.5);
        unclipped.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        let mut clipped = Trainer::new(1, 0.5);
        clipped.out_clip_norm = Some(1.0); // far below any real output-layer gradient norm
        clipped.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        assert_eq!(clipped.out_clip_count, 1, "the tiny threshold must trigger");
        assert_eq!(clipped.ft_clip_count, 0);
        assert_eq!(clipped.l2_clip_count, 0);
        // Output layer's applied update must shrink...
        assert!(clipped.out_update_norm_sum < unclipped.out_update_norm_sum);
        // ...while FT/L2 -- the whole point of output-*only* clipping --
        // must be completely unaffected, byte-identical to the unclipped run.
        assert_eq!(clipped.ft_update_norm_sum, unclipped.ft_update_norm_sum);
        assert_eq!(clipped.l2_update_norm_sum, unclipped.l2_update_norm_sum);
        // Diagnostics still record the natural (pre-clip) output-layer norm.
        assert_eq!(
            clipped.out_grad_norm_values[0],
            unclipped.out_grad_norm_values[0]
        );
        // ...and the after-clip mean reflects the cap actually applied.
        assert!(clipped.out_grad_norm_after_sum < unclipped.out_grad_norm_after_sum);
    }

    #[test]
    fn diagnostic_freeze_layer_unset_is_byte_identical_to_no_freeze() {
        // Regression for the "flag omitted" no-op guarantee: setting
        // `diagnostic_freeze_until_position` alone, with `diagnostic_freeze_layer`
        // left at its `None` default, must never freeze anything -- the layer
        // choice, not the position bound alone, is what activates freezing.
        let board = Board::startpos();

        let mut baseline = Trainer::new(1, 0.5);
        baseline.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        let mut untouched = Trainer::new(1, 0.5);
        untouched.diagnostic_freeze_until_position = 999; // layer left None
        untouched.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        assert_eq!(baseline.weights.ft, untouched.weights.ft);
        assert_eq!(baseline.weights.l2, untouched.weights.l2);
        assert_eq!(baseline.weights.out, untouched.weights.out);
        assert_eq!(baseline.total_loss, untouched.total_loss);
    }

    #[test]
    fn train_position_freeze_layer_l2_leaves_l2_unchanged_but_ft_and_out_still_update() {
        // Proves both halves of the L2-freeze contract at once: the frozen
        // layer's own params must not move, and -- critically, this is NOT
        // stop-gradient -- FT (upstream of L2) must still receive a real
        // gradient through L2's now-fixed weights and actually update.
        let board = Board::startpos();
        let mut trainer = Trainer::new(1, 0.5);
        trainer.diagnostic_freeze_layer = Some(FreezeLayer::L2);
        trainer.diagnostic_freeze_until_position = 10;

        let l2_before = trainer.weights.l2.clone();
        let l2_bias_before = trainer.weights.l2_bias.clone();
        let ft_before = trainer.weights.ft.clone();
        let out_before = trainer.weights.out.clone();

        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        assert_eq!(
            trainer.weights.l2, l2_before,
            "frozen L2 weights must not move"
        );
        assert_eq!(
            trainer.weights.l2_bias, l2_bias_before,
            "frozen L2 bias must not move"
        );
        assert_ne!(
            trainer.weights.ft, ft_before,
            "FT must still update -- gradient must flow through the frozen L2 weights, not be cut"
        );
        assert_ne!(
            trainer.weights.out, out_before,
            "Output must still update normally, unaffected by an L2 freeze"
        );
    }

    #[test]
    fn train_position_freeze_layer_ft_leaves_ft_unchanged_but_l2_and_out_still_update() {
        let board = Board::startpos();
        let mut trainer = Trainer::new(1, 0.5);
        trainer.diagnostic_freeze_layer = Some(FreezeLayer::Ft);
        trainer.diagnostic_freeze_until_position = 10;

        let ft_before = trainer.weights.ft.clone();
        let ft_bias_before = trainer.weights.ft_bias.clone();
        let l2_before = trainer.weights.l2.clone();
        let out_before = trainer.weights.out.clone();

        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        assert_eq!(
            trainer.weights.ft, ft_before,
            "frozen FT weights must not move"
        );
        assert_eq!(
            trainer.weights.ft_bias, ft_bias_before,
            "frozen FT bias must not move"
        );
        assert_ne!(
            trainer.weights.l2, l2_before,
            "L2 must still update normally, unaffected by an FT freeze"
        );
        assert_ne!(
            trainer.weights.out, out_before,
            "Output must still update normally, unaffected by an FT freeze"
        );
    }

    #[test]
    fn train_position_freeze_layer_out_leaves_out_unchanged_but_ft_and_l2_still_update() {
        // The arm tied to the standing output→L2 backprop hypothesis: if Out
        // were wrongly implemented as stop-gradient, L2/FT would see zero
        // gradient and never move -- this proves they still do.
        let board = Board::startpos();
        let mut trainer = Trainer::new(1, 0.5);
        trainer.diagnostic_freeze_layer = Some(FreezeLayer::Out);
        trainer.diagnostic_freeze_until_position = 10;

        let out_before = trainer.weights.out.clone();
        let out_bias_before = trainer.weights.out_bias;
        let l2_before = trainer.weights.l2.clone();
        let ft_before = trainer.weights.ft.clone();

        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        assert_eq!(
            trainer.weights.out, out_before,
            "frozen Output weights must not move"
        );
        assert_eq!(
            trainer.weights.out_bias, out_bias_before,
            "frozen Output bias must not move"
        );
        assert_ne!(
            trainer.weights.l2, l2_before,
            "L2 must still update -- gradient must flow through the frozen Out weights, not be cut"
        );
        assert_ne!(
            trainer.weights.ft, ft_before,
            "FT must still update -- gradient must flow all the way through, not be cut"
        );
    }

    #[test]
    fn diagnostic_freeze_layer_resumes_updating_once_the_position_bound_is_passed() {
        let board = Board::startpos();
        let mut trainer = Trainer::new(1, 0.5);
        trainer.diagnostic_freeze_layer = Some(FreezeLayer::L2);
        trainer.diagnostic_freeze_until_position = 1; // only position 1 is frozen

        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);
        assert_eq!(trainer.l2_sample_count, 1);
        let l2_after_frozen_position = trainer.weights.l2.clone();

        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);
        assert_eq!(trainer.l2_sample_count, 2);
        assert_ne!(
            trainer.weights.l2, l2_after_frozen_position,
            "position 2 is past diagnostic_freeze_until_position=1, L2 must resume updating"
        );
    }

    #[test]
    fn diagnostic_freeze_from_position_unset_is_byte_identical_to_no_freeze() {
        // Mirrors `diagnostic_freeze_layer_unset_is_byte_identical_to_no_freeze`:
        // setting `diagnostic_freeze_from_position` alone, with `diagnostic_freeze_layer`
        // left at its `None` default, must never freeze anything.
        let board = Board::startpos();

        let mut baseline = Trainer::new(1, 0.5);
        baseline.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        let mut untouched = Trainer::new(1, 0.5);
        untouched.diagnostic_freeze_from_position = 1;
        untouched.diagnostic_freeze_until_position = 999; // layer left None
        untouched.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);

        assert_eq!(baseline.weights.ft, untouched.weights.ft);
        assert_eq!(baseline.weights.l2, untouched.weights.l2);
        assert_eq!(baseline.weights.out, untouched.weights.out);
        assert_eq!(baseline.total_loss, untouched.total_loss);
    }

    #[test]
    fn diagnostic_freeze_window_only_freezes_between_from_and_until_positions() {
        // The windowed-freeze contract this test proves: positions before
        // `from_position` update normally, positions inside [from, until]
        // are frozen, positions after `until` resume updating -- a closed
        // window, not just an from-the-start cutoff.
        let board = Board::startpos();
        let mut trainer = Trainer::new(1, 0.5);
        trainer.diagnostic_freeze_layer = Some(FreezeLayer::Ft);
        trainer.diagnostic_freeze_from_position = 2;
        trainer.diagnostic_freeze_until_position = 3;

        // Position 1: before the window, FT must update normally.
        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);
        assert_eq!(trainer.l2_sample_count, 1);
        let ft_after_position_1 = trainer.weights.ft.clone();

        // Position 2: inside the window, FT must be frozen.
        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);
        assert_eq!(trainer.l2_sample_count, 2);
        assert_eq!(
            trainer.weights.ft, ft_after_position_1,
            "position 2 is inside [from=2, until=3], FT must stay frozen"
        );
        let ft_after_position_2 = trainer.weights.ft.clone();

        // Position 3: still inside the window, FT must still be frozen.
        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);
        assert_eq!(trainer.l2_sample_count, 3);
        assert_eq!(
            trainer.weights.ft, ft_after_position_2,
            "position 3 is inside [from=2, until=3], FT must stay frozen"
        );

        // Position 4: past the window, FT must resume updating.
        trainer.train_position(&board, -600.0, 1.0, -600.0, None, 0, GameResult::Unknown);
        assert_eq!(trainer.l2_sample_count, 4);
        assert_ne!(
            trainer.weights.ft, ft_after_position_2,
            "position 4 is past diagnostic_freeze_until_position=3, FT must resume updating"
        );
    }

    #[test]
    fn train_position_wdl_component_only_accumulates_when_target_present() {
        let mut trainer = Trainer::new(1, 0.5);
        let board = Board::startpos();

        // wdl_target = None (e.g. GameResult::Unknown, or the positions
        // path, which has no result signal at all) -- wdl_component must
        // not accumulate, since there's nothing to compute it against.
        trainer.train_position(&board, 10.0, 1.0, 10.0, None, 0, GameResult::Unknown);
        assert_eq!(trainer.wdl_component_count, 0);
        assert_eq!(trainer.wdl_component_sum, 0.0);

        // wdl_target = Some(_) -- both cp_component (vs eval_teacher) and
        // wdl_component (vs wdl_target) must accumulate, using the RAW
        // components, not the blended `teacher` passed as the actual
        // gradient target.
        trainer.train_position(&board, 5.0, 1.0, 20.0, Some(-30.0), 0, GameResult::Unknown);
        assert_eq!(trainer.wdl_component_count, 1);
        assert!(trainer.wdl_component_sum > 0.0);
        assert!(trainer.cp_component_sum > 0.0);
    }

    #[test]
    fn train_position_records_exactly_one_grad_norm_sample_per_call() {
        let mut trainer = Trainer::new(1, 0.5);
        let board = Board::startpos();
        trainer.train_position(&board, 10.0, 1.0, 10.0, None, 0, GameResult::Unknown);
        trainer.train_position(&board, 10.0, 1.0, 10.0, None, 0, GameResult::Unknown);
        assert_eq!(trainer.global_grad_norm_values.len(), 2);
        assert!(
            trainer
                .global_grad_norm_values
                .iter()
                .all(|&g| g >= 0.0 && g.is_finite())
        );
        assert!(trainer.ft_grad_norm_sum_sq >= 0.0);
    }

    #[test]
    fn trace_positions_snapshots_exactly_the_requested_points() {
        let mut trainer = Trainer::new(1, 0.5);
        trainer.trace_positions = [2u64, 5].into_iter().collect();
        let board = Board::startpos();
        for _ in 0..6 {
            trainer.train_position(&board, 10.0, 1.0, 10.0, None, 0, GameResult::Unknown);
        }
        let indices: Vec<u64> = trainer
            .trace_snapshots
            .iter()
            .map(|s| s.position_index)
            .collect();
        assert_eq!(indices, vec![2, 5]);
        // Sample counts are monotonically increasing and match the
        // requested position -- a snapshot at index N reflects exactly N
        // positions processed so far, not the whole epoch.
        for snapshot in &trainer.trace_snapshots {
            assert_eq!(snapshot.l2.bias.len(), L2);
            assert_eq!(snapshot.ft.bias.len(), L1);
        }
        // Requesting `0` is a no-op (see `Trainer::trace_positions`'s doc
        // comment) -- never reached since the first snapshot opportunity
        // is after position 1 completes.
        let mut trainer2 = Trainer::new(1, 0.5);
        trainer2.trace_positions = [0u64].into_iter().collect();
        trainer2.train_position(&board, 10.0, 1.0, 10.0, None, 0, GameResult::Unknown);
        assert!(trainer2.trace_snapshots.is_empty());
    }

    #[test]
    fn trace_positions_omitted_writes_no_snapshots() {
        // Default-constructed Trainer has an empty `trace_positions` --
        // confirms the feature is off unless explicitly opted into, same
        // discipline as every other diagnostic flag this session.
        let mut trainer = Trainer::new(1, 0.5);
        assert!(trainer.trace_positions.is_empty());
        let board = Board::startpos();
        for _ in 0..10 {
            trainer.train_position(&board, 10.0, 1.0, 10.0, None, 0, GameResult::Unknown);
        }
        assert!(trainer.trace_snapshots.is_empty());
    }

    #[test]
    fn shuffled_order_is_a_permutation() {
        let order = shuffled_order(500, 42);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..500).collect::<Vec<_>>());
    }

    #[test]
    fn shuffled_order_is_deterministic_for_the_same_seed() {
        assert_eq!(shuffled_order(200, 7), shuffled_order(200, 7));
    }

    #[test]
    fn shuffled_order_differs_across_seeds() {
        assert_ne!(shuffled_order(200, 1), shuffled_order(200, 2));
    }

    #[test]
    fn shuffled_order_handles_zero_and_one() {
        assert_eq!(shuffled_order(0, 42), Vec::<usize>::new());
        assert_eq!(shuffled_order(1, 42), vec![0]);
    }

    #[test]
    fn l2_preactivation_gradient_matches_doutput_times_out_weight_times_clippedrelu_derivative() {
        // dL/dL2_preactivation[o] = dL/dOutput * out_weight[o] * ClippedReLU'(l2_acc[o]),
        // where ClippedReLU' is 1 inside (0, 127) and 0 outside (dead/saturated) --
        // the exact formula `train_position`'s own backward pass uses
        // (`d_l2_acc[o] = d_output * self.weights.out[o]` gated by the
        // `l2_acc[o] > 0.0 && l2_acc[o] < 127.0` check just above it). This
        // test reconstructs the RHS independently (from `forward`'s own
        // score plus the pre-update `out` weights) and checks it against
        // `l2_dacc_sum` -- exactly `d_l2_acc` for this single call, since
        // the accumulator starts at 0 and this is the only call made.
        // Because ClippedReLU' only ever takes the value 0 or 1, "matches
        // the unclamped product exactly, or is exactly 0" *is* the full
        // statement of the formula -- no separate read of `l2_acc`'s gate
        // state is needed to state the identity precisely.
        let mut trainer = Trainer::new(3, 0.5);
        let board = Board::startpos();
        let teacher = 42.0;
        let weight = 1.0;

        let score_before = trainer.forward(&board);
        let out_before = trainer.weights.out.clone();
        let d_output_expected = weight * 2.0 * (score_before - teacher) / 64.0;

        trainer.train_position(
            &board,
            teacher,
            weight,
            teacher,
            None,
            0,
            GameResult::Unknown,
        );

        for o in 0..L2 {
            let actual = trainer.l2_dacc_sum[o];
            let unclamped = (d_output_expected * out_before[o]) as f64;
            let matches_unclamped = (actual - unclamped).abs() < 1e-4;
            let is_zero = actual == 0.0;
            assert!(
                matches_unclamped || is_zero,
                "neuron {o}: l2_dacc_sum={actual} does not match d_output*out_weight={unclamped} and isn't 0 (dead/saturated)"
            );
        }
    }

    #[test]
    fn cp_wdl_grad_trace_blended_gradient_is_the_expected_weighted_sum() {
        // The blended teacher's gradient must equal lambda*CP-only +
        // (1-lambda)*WDL-only at every neuron -- the mathematical property
        // the whole decomposition rests on (see the module doc comment's
        // "single blended teacher... deliberate, not a shortcut" note).
        // Checked against the *real* blended-gradient accumulator
        // (`l2_dacc_sum`/`ft_dacc_sum`, already used by `--trace-positions`
        // and never touched by this flag), not a second independent
        // computation, so this catches decomposition bugs directly.
        let mut trainer = Trainer::new(1, 0.5);
        trainer.cp_wdl_grad_trace = true;
        trainer.trace_positions = [3u64].into_iter().collect();
        let board = Board::startpos();
        let lambda = 0.7f32;
        let eval_teacher = 40.0f32;
        let wdl_target = -120.0f32;
        let teacher = lambda * eval_teacher + (1.0 - lambda) * wdl_target;

        for _ in 0..3 {
            trainer.train_position(
                &board,
                teacher,
                1.0,
                eval_teacher,
                Some(wdl_target),
                0,
                GameResult::Unknown,
            );
        }

        // Target/prediction/residual/dL-dOutput fields are populated and
        // sane -- `eval_teacher`/`wdl_target` are literally constant across
        // all 3 calls here, so their accumulated means must match exactly.
        let cp_wdl = trainer.trace_snapshots[0]
            .cp_wdl
            .as_ref()
            .expect("cp_wdl populated");
        assert!((cp_wdl.cp_target_mean - eval_teacher as f64).abs() < 1e-6);
        assert!((cp_wdl.wdl_target_mean - wdl_target as f64).abs() < 1e-6);
        assert!(cp_wdl.prediction_mean.is_finite());
        assert!(cp_wdl.cp_residual_std.is_finite() && cp_wdl.cp_residual_std >= 0.0);
        assert!(cp_wdl.wdl_residual_std.is_finite() && cp_wdl.wdl_residual_std >= 0.0);
        assert!(cp_wdl.cp_d_output_mean.is_finite());
        assert!(cp_wdl.wdl_d_output_mean.is_finite());

        for o in 0..L2 {
            let expected = lambda as f64 * trainer.l2_cp_dacc_sum[o]
                + (1.0 - lambda) as f64 * trainer.l2_wdl_dacc_sum[o];
            assert!(
                (trainer.l2_dacc_sum[o] - expected).abs() < 1e-3,
                "l2 neuron {o}: blended={} expected={}",
                trainer.l2_dacc_sum[o],
                expected
            );
        }
        for j in 0..L1 {
            let expected = lambda as f64 * trainer.ft_cp_dacc_sum[j]
                + (1.0 - lambda) as f64 * trainer.ft_wdl_dacc_sum[j];
            assert!(
                (trainer.ft_dacc_sum[j] - expected).abs() < 1e-3,
                "ft neuron {j}: blended={} expected={}",
                trainer.ft_dacc_sum[j],
                expected
            );
        }
    }

    #[test]
    fn cp_wdl_grad_trace_does_not_alter_training_state() {
        // The diagnostic backward passes must be pure side computations:
        // enabling the flag must not change a single trained weight, Adam
        // moment, or the RNG-derived randomness anything downstream would
        // see -- only new diagnostic fields should differ. Compares two
        // Trainers, identical seed/inputs, one with the flag on.
        let board = Board::startpos();
        let lambda = 0.7f32;
        let eval_teacher = 40.0f32;
        let wdl_target = -120.0f32;
        let teacher = lambda * eval_teacher + (1.0 - lambda) * wdl_target;

        let mut plain = Trainer::new(1, 0.5);
        let mut traced = Trainer::new(1, 0.5);
        traced.cp_wdl_grad_trace = true;

        for _ in 0..5 {
            plain.train_position(
                &board,
                teacher,
                1.0,
                eval_teacher,
                Some(wdl_target),
                0,
                GameResult::Unknown,
            );
            traced.train_position(
                &board,
                teacher,
                1.0,
                eval_teacher,
                Some(wdl_target),
                0,
                GameResult::Unknown,
            );
        }

        assert_eq!(
            plain.weights.snapshot_params(),
            traced.weights.snapshot_params()
        );
        assert_eq!(plain.total_loss, traced.total_loss);
        assert_eq!(plain.l2_dacc_sum, traced.l2_dacc_sum);
        assert_eq!(plain.ft_grad_norm_sum, traced.ft_grad_norm_sum);
    }

    #[test]
    fn sample_grad_trace_records_expected_fields_and_cosine_semantics() {
        let mut trainer = Trainer::new(1, 0.5);
        trainer.sample_grad_trace_limit = 3;
        let board = Board::startpos();

        trainer.train_position(&board, 10.0, 1.0, 10.0, None, 42, GameResult::WhiteWin);
        trainer.train_position(&board, 10.0, 1.0, 10.0, None, 42, GameResult::WhiteWin);

        assert_eq!(trainer.sample_grad_records.len(), 2);
        let first = &trainer.sample_grad_records[0];
        let second = &trainer.sample_grad_records[1];

        assert_eq!(first.game_id, 42);
        assert_eq!(first.game_result, "WhiteWin");
        assert_eq!(first.position_index, 1);
        assert_eq!(second.position_index, 2);
        // First recorded position has no predecessor and no running mean yet.
        assert_eq!(first.cosine_prev, None);
        assert_eq!(first.cosine_running_mean, None);
        // Second position has both -- identical repeated positions/weights
        // between calls (only Adam's tiny first step separates them), so
        // the gradient direction should be highly self-similar, not None.
        assert!(second.cosine_prev.is_some());
        assert!(second.cosine_running_mean.is_some());
        assert_eq!(first.l2_gate.len(), L2);
    }

    #[test]
    fn sample_grad_trace_does_not_alter_training_state() {
        // Same guarantee as `cp_wdl_grad_trace_does_not_alter_training_state`,
        // for `--sample-grad-trace`: recording per-position gradient-
        // correlation records is a pure side computation over already-
        // computed forward/backward state, and must not change a single
        // trained weight, Adam moment, or the blended-gradient accumulators
        // anything downstream would see.
        let board = Board::startpos();
        let lambda = 0.7f32;
        let eval_teacher = 40.0f32;
        let wdl_target = -120.0f32;
        let teacher = lambda * eval_teacher + (1.0 - lambda) * wdl_target;

        let mut plain = Trainer::new(1, 0.5);
        let mut traced = Trainer::new(1, 0.5);
        traced.sample_grad_trace_limit = 5;

        for i in 0..8 {
            plain.train_position(
                &board,
                teacher,
                1.0,
                eval_teacher,
                Some(wdl_target),
                7,
                GameResult::BlackWin,
            );
            traced.train_position(
                &board,
                teacher,
                1.0,
                eval_teacher,
                Some(wdl_target),
                7,
                GameResult::BlackWin,
            );
            // Sanity: the trace actually did something up to the limit,
            // and stopped recording past it (otherwise this test could
            // pass vacuously with a no-op flag).
            let expected_records = (i + 1).min(5);
            assert_eq!(traced.sample_grad_records.len(), expected_records);
        }

        assert_eq!(
            plain.weights.snapshot_params(),
            traced.weights.snapshot_params()
        );
        assert_eq!(plain.total_loss, traced.total_loss);
        assert_eq!(plain.l2_dacc_sum, traced.l2_dacc_sum);
        assert_eq!(plain.ft_grad_norm_sum, traced.ft_grad_norm_sum);
    }
}
