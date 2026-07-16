//! Per-epoch training diagnostics — pure functions over counters the
//! trainer accumulates during an epoch, or over saved weights.
//!
//! The only prior "diagnostic" this project had was a one-off manual read
//! of saved weight files that found the 2026-07-09 capacity-collapse bug
//! (every FT/L2 row a single repeated scalar). These functions turn that
//! kind of check into a routine per-epoch printout instead of something
//! only found by accident, months after the fact.

use sekirei_core::nnue::NnueWeights;

#[derive(Debug, Clone)]
pub struct EpochDiagnostics {
    /// Whole-parameter-vector L2 distance from the previous epoch's
    /// snapshot. `None` on the first epoch (no previous snapshot exists).
    pub param_update_norm: Option<f32>,
    pub ft_active_ratio: f32,
    pub ft_saturation_ratio: f32,
    pub output_mean: f64,
    pub output_std: f64,
    pub quantized_ft_zero_ratio: f32,
    /// Fraction of L2 neurons that fired (post-activation > 0) *at least
    /// once* during the epoch. Renamed from `l2_active_ratio` -- a
    /// set-membership measure, distinct from the frequency-based
    /// `l2_activation_frequency_*` fields below (see
    /// `l2_saturation_probe.rs`'s doc comment for why the distinction
    /// matters: this being equal to `l2_ever_saturated_ratio` means every
    /// ever-active neuron also touches the ceiling at least once, not that
    /// it's pinned there).
    pub l2_ever_active_ratio: f32,
    pub l2_ever_saturated_ratio: f32,
    /// Count of L2 neurons with post-activation == 0 for *every* sample
    /// this epoch (activation frequency == 0.0).
    pub l2_dead_neurons: usize,
    pub l2_activation_frequency_mean: f32,
    pub l2_saturation_frequency_mean: f32,
    pub l2_activation_frequency_per_neuron: Vec<f32>,
    pub l2_saturation_frequency_per_neuron: Vec<f32>,
    /// Percentiles of L2 pre-clamp values, pooled across all neurons and
    /// samples (not per-neuron).
    pub l2_preactivation_p01: f32,
    pub l2_preactivation_p10: f32,
    pub l2_preactivation_p50: f32,
    pub l2_preactivation_p90: f32,
    pub l2_preactivation_p99: f32,
    pub l2_bias_per_neuron: Vec<f32>,
    pub l2_row_weight_norm_per_neuron: Vec<f32>,
    /// Output-layer weight vector norm and bias -- for tracking whether the
    /// final linear layer itself is what's driving output-scale runaway,
    /// as opposed to L2/FT.
    pub output_weight_norm: f32,
    pub output_bias: f32,
    // Per-position gradient-norm mean/std, one pair per layer (FT bundles
    // its bias, etc. -- see `Trainer`'s field docs). Distinct from the
    // update-norm fields below: under Adam, a smaller gradient doesn't
    // imply a smaller applied step.
    pub ft_grad_norm_mean: f64,
    pub ft_grad_norm_std: f64,
    pub l2_grad_norm_mean: f64,
    pub l2_grad_norm_std: f64,
    pub out_grad_norm_mean: f64,
    pub out_grad_norm_std: f64,
    /// Percentiles of the *global* (whole-network) per-position gradient
    /// norm -- the quantity a `--grad-clip-norm` threshold would act on.
    /// p95/p99 are the ones a clip-value choice should be based on, not
    /// the mean (clipping targets the tail, not the typical case).
    pub global_grad_norm_p50: f32,
    pub global_grad_norm_p90: f32,
    pub global_grad_norm_p95: f32,
    pub global_grad_norm_p99: f32,
    // Per-position *applied update* norm mean/std, one pair per layer --
    // the actual step Adam takes, as opposed to the raw gradient above.
    pub ft_update_norm_mean: f64,
    pub ft_update_norm_std: f64,
    pub l2_update_norm_mean: f64,
    pub l2_update_norm_std: f64,
    pub out_update_norm_mean: f64,
    pub out_update_norm_std: f64,
    /// Mean/std of the (possibly WDL-blended) training target -- within-run
    /// monitoring only, not comparable across different `wdl_lambda` runs
    /// (unlike the cp/wdl components below, which are).
    pub target_mean: f64,
    pub target_std: f64,
    /// Pearson correlation between prediction and the *raw* eval component
    /// (not the blended target), so this stays comparable across runs at
    /// different λ -- same rationale as `valid_cp_mse`.
    pub pred_eval_correlation: f64,
    /// Training-side loss split into its CP/WDL components, computed
    /// against the same raw components `ValidStats`'s `cp_mse`/`wdl_loss`
    /// use -- comparable across `wdl_lambda`, and answers whether λ=0.7 is
    /// a genuinely better-fitting auxiliary signal or just a smaller/
    /// smoother combined objective that masks a worse cp fit. Never used
    /// for the actual gradient; see `Trainer::train_position`.
    pub train_cp_component: f64,
    pub train_wdl_component: Option<f64>,
    /// Positions this epoch whose gradient exceeded `--grad-clip-norm` and
    /// got scaled down. Always 0 when clipping is off.
    pub grad_clip_count: u64,
    /// Per-layer clip trigger *rates* (count / total_count), for the
    /// independent `--ft-clip-norm`/`--l2-clip-norm`/`--out-clip-norm`
    /// thresholds. Always 0.0 when that layer's threshold is unset -- in
    /// particular, output-only clipping (only `out_clip_norm` set) always
    /// reports `ft_clip_trigger_rate == l2_clip_trigger_rate == 0.0`,
    /// proving those layers were untouched, not just assuming it.
    pub ft_clip_trigger_rate: f64,
    pub l2_clip_trigger_rate: f64,
    pub out_clip_trigger_rate: f64,
    /// Percentiles of the *output-layer* per-position gradient norm -- the
    /// quantity `--out-clip-norm` should be chosen from (its own
    /// distribution, not the global one `--grad-clip-norm` uses, since
    /// `out`'s raw scale dominates the global norm and the two
    /// distributions are related but not identical).
    pub out_grad_norm_p95: f32,
    pub out_grad_norm_p99: f32,
    /// Mean/std of the output-layer gradient norm *after* per-layer
    /// clipping -- pairs with `out_grad_norm_mean`/`std` above (which stay
    /// pre-clip, "before") to show how much clipping actually moved the
    /// distribution. Equal to the "before" pair when `out_clip_norm` is
    /// unset or never triggers.
    pub out_grad_norm_after_mean: f64,
    pub out_grad_norm_after_std: f64,
}

/// One layer's (L2 or FT) per-neuron state at one `--trace-positions`
/// snapshot point. Every field is per-neuron (length L2=32 or L1=256) and,
/// except `weight_row_norm`/`bias` (current parameter state, no history
/// needed), cumulative *since epoch start* -- the same semantic
/// `EpochDiagnostics`'s own epoch-end fields already use, just read at an
/// intermediate point instead of only at the end. `weighted_input_p*` is
/// only populated for L2 (see `Trainer::l2_weighted_input_values`'s doc
/// comment) -- left as empty vecs for FT.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TraceLayerSnapshot {
    pub preactivation_p10: Vec<f32>,
    pub preactivation_p50: Vec<f32>,
    pub preactivation_p90: Vec<f32>,
    pub weighted_input_p10: Vec<f32>,
    pub weighted_input_p50: Vec<f32>,
    pub weighted_input_p90: Vec<f32>,
    pub dead_frequency: Vec<f32>,
    pub saturation_frequency: Vec<f32>,
    pub weight_row_norm: Vec<f32>,
    pub bias: Vec<f32>,
    /// Mean of `d_{layer}_acc[o]` (gradient of the loss w.r.t. this
    /// neuron's own pre-activation) across positions so far this epoch --
    /// signed, so a consistently one-directional push shows as a mean far
    /// from 0, while an oscillating one cancels toward 0.
    pub gradient_mean: Vec<f32>,
    /// RMS of the same per-position gradient -- unlike the signed mean,
    /// this can't cancel: `mean ≈ 0` but `gradient_rms` large means the
    /// neuron is being pushed hard in alternating directions, not left
    /// alone.
    pub gradient_rms: Vec<f32>,
    /// `|pos_count - neg_count| / (pos_count + neg_count)` of that
    /// gradient's sign across positions so far -- 1.0 means every position
    /// pushed this neuron the same direction, 0.0 means an even split.
    pub gradient_sign_consistency: Vec<f32>,
    /// `sqrt(sum of squared applied Adam deltas to this neuron's bias)`,
    /// cumulative since epoch start.
    pub update_norm: Vec<f32>,
}

/// One `--trace-positions` snapshot: L2 and FT's joint per-neuron state
/// after `position_index` positions have been fully processed (forward +
/// backward + Adam step) since epoch start.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TraceSnapshot {
    pub position_index: u64,
    pub l2: TraceLayerSnapshot,
    pub ft: TraceLayerSnapshot,
    /// Mean/std of the concatenated 2×L1-wide FT-output vector feeding L2,
    /// across positions so far this epoch.
    pub l2_input_norm_mean: f64,
    pub l2_input_norm_std: f64,
    /// Mean/std of FT's own post-activation output, pooled across both
    /// perspectives and all L1 neurons (layer-wide, not per-neuron),
    /// across positions so far this epoch.
    pub ft_output_mean: f64,
    pub ft_output_std: f64,
    /// `--cp-wdl-grad-trace`'s CP-vs-WDL gradient decomposition -- `None`
    /// when the flag is off (the default) or this position had no WDL
    /// signal to decompose against.
    pub cp_wdl: Option<CpWdlTrace>,
}

/// One layer's (L2 or FT) per-neuron CP-only vs. WDL-only gradient
/// comparison, cumulative since epoch start -- same cadence and semantic
/// as `TraceLayerSnapshot`'s own gradient fields, just split by teacher
/// signal instead of using the blended one.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CpWdlLayerTrace {
    pub cp_gradient_mean: Vec<f32>,
    pub wdl_gradient_mean: Vec<f32>,
    pub cp_gradient_sign_consistency: Vec<f32>,
    pub wdl_gradient_sign_consistency: Vec<f32>,
    /// Per-neuron cosine similarity between the CP-only and WDL-only
    /// per-position gradient, treating "positions so far this epoch" as
    /// the vector dimension: +1 means the two signals always push this
    /// neuron the same direction, -1 means they always oppose, 0 means
    /// uncorrelated (or the neuron never received a nonzero gradient from
    /// either signal).
    pub cosine_similarity: Vec<f32>,
}

/// Whole-layer gradient RMS (FT/L2/output), split by teacher signal --
/// the layer-wide counterpart to `CpWdlLayerTrace`'s per-neuron fields.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CpWdlTrace {
    pub l2: CpWdlLayerTrace,
    pub ft: CpWdlLayerTrace,
    pub cp_ft_grad_rms: f64,
    pub wdl_ft_grad_rms: f64,
    pub cp_l2_grad_rms: f64,
    pub wdl_l2_grad_rms: f64,
    pub cp_out_grad_rms: f64,
    pub wdl_out_grad_rms: f64,
    /// Target/prediction/residual/dL-dOutput distributions, all scoped to
    /// the same wdl-having position subset the fields above already use --
    /// explains *why* the gradient-scale fields differ, not just that they
    /// do. `prediction_*` is shared (the network has one output regardless
    /// of which teacher signal is being evaluated against it), everything
    /// else is split by signal.
    pub cp_target_mean: f64,
    pub cp_target_std: f64,
    pub wdl_target_mean: f64,
    pub wdl_target_std: f64,
    pub prediction_mean: f64,
    pub prediction_std: f64,
    /// Signed residual (`score - target`) mean/std -- distinct from the
    /// squared-error (MSE) accumulators used elsewhere, which can't tell
    /// "consistently offset" from "large but symmetric" error.
    pub cp_residual_mean: f64,
    pub cp_residual_std: f64,
    pub wdl_residual_mean: f64,
    pub wdl_residual_std: f64,
    pub cp_d_output_mean: f64,
    pub cp_d_output_std: f64,
    pub wdl_d_output_mean: f64,
    pub wdl_d_output_std: f64,
}

/// Sign consistency of a per-neuron gradient accumulator:
/// `|pos-neg|/(pos+neg)`, 0.0 when a neuron never received a nonzero
/// gradient (avoids a 0/0 division).
fn sign_consistency(pos_count: u64, neg_count: u64) -> f32 {
    let total = pos_count + neg_count;
    if total == 0 {
        return 0.0;
    }
    (pos_count as f32 - neg_count as f32).abs() / total as f32
}

/// Builds one layer's `TraceLayerSnapshot` from its accumulators.
/// `weighted_input_values` is `&[]` for FT (no weighted-input/bias split
/// tracked for that layer, see `TraceLayerSnapshot`'s doc comment).
#[allow(clippy::too_many_arguments)]
pub fn build_trace_layer_snapshot(
    values: &[Vec<f32>],
    weighted_input_values: &[Vec<f32>],
    zero_count: &[u64],
    sat_count: &[u64],
    sample_count: u64,
    weight_row_norm: Vec<f32>,
    bias: Vec<f32>,
    dacc_sum: &[f64],
    dacc_sq_sum: &[f64],
    dacc_pos_count: &[u64],
    dacc_neg_count: &[u64],
    bias_update_sq_sum: &[f64],
) -> TraceLayerSnapshot {
    let n = values.len();
    let mut preactivation_p10 = Vec::with_capacity(n);
    let mut preactivation_p50 = Vec::with_capacity(n);
    let mut preactivation_p90 = Vec::with_capacity(n);
    for v in values {
        let p = percentiles(v, &[0.10, 0.50, 0.90]);
        preactivation_p10.push(p[0]);
        preactivation_p50.push(p[1]);
        preactivation_p90.push(p[2]);
    }
    let mut weighted_input_p10 = Vec::new();
    let mut weighted_input_p50 = Vec::new();
    let mut weighted_input_p90 = Vec::new();
    for v in weighted_input_values {
        let p = percentiles(v, &[0.10, 0.50, 0.90]);
        weighted_input_p10.push(p[0]);
        weighted_input_p50.push(p[1]);
        weighted_input_p90.push(p[2]);
    }
    let gradient_mean: Vec<f32> = dacc_sum
        .iter()
        .map(|&s| {
            if sample_count > 0 {
                (s / sample_count as f64) as f32
            } else {
                0.0
            }
        })
        .collect();
    let gradient_rms: Vec<f32> = dacc_sq_sum
        .iter()
        .map(|&s| {
            if sample_count > 0 {
                (s / sample_count as f64).sqrt() as f32
            } else {
                0.0
            }
        })
        .collect();
    let gradient_sign_consistency: Vec<f32> = dacc_pos_count
        .iter()
        .zip(dacc_neg_count)
        .map(|(&p, &n)| sign_consistency(p, n))
        .collect();
    let update_norm: Vec<f32> = bias_update_sq_sum
        .iter()
        .map(|&s| (s.sqrt()) as f32)
        .collect();
    let dead_frequency: Vec<f32> = if sample_count == 0 {
        vec![0.0; zero_count.len()]
    } else {
        zero_count
            .iter()
            .map(|&z| z as f32 / sample_count as f32)
            .collect()
    };
    TraceLayerSnapshot {
        preactivation_p10,
        preactivation_p50,
        preactivation_p90,
        weighted_input_p10,
        weighted_input_p50,
        weighted_input_p90,
        dead_frequency,
        saturation_frequency: l2_saturation_frequency_per_neuron(sat_count, sample_count),
        weight_row_norm,
        bias,
        gradient_mean,
        gradient_rms,
        gradient_sign_consistency,
        update_norm,
    }
}

/// Cosine similarity between two per-position accumulator pairs (dot
/// product sum and each side's own sum-of-squares), without ever storing
/// full per-position history -- 0.0 when either side never received a
/// nonzero gradient (avoids a 0/0 division), matching `sign_consistency`'s
/// own convention.
fn cosine_similarity(dot_sum: f64, a_sq_sum: f64, b_sq_sum: f64) -> f32 {
    let denom = (a_sq_sum * b_sq_sum).sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    (dot_sum / denom) as f32
}

/// Builds one layer's `CpWdlLayerTrace` from `--cp-wdl-grad-trace`'s
/// per-neuron accumulators.
#[allow(clippy::too_many_arguments)]
pub fn build_cp_wdl_layer_trace(
    cp_sum: &[f64],
    cp_sq_sum: &[f64],
    cp_pos_count: &[u64],
    cp_neg_count: &[u64],
    wdl_sum: &[f64],
    wdl_sq_sum: &[f64],
    wdl_pos_count: &[u64],
    wdl_neg_count: &[u64],
    dot_sum: &[f64],
    sample_count: u64,
) -> CpWdlLayerTrace {
    let mean = |sum: &[f64]| -> Vec<f32> {
        sum.iter()
            .map(|&s| {
                if sample_count > 0 {
                    (s / sample_count as f64) as f32
                } else {
                    0.0
                }
            })
            .collect()
    };
    let n = cp_sum.len();
    CpWdlLayerTrace {
        cp_gradient_mean: mean(cp_sum),
        wdl_gradient_mean: mean(wdl_sum),
        cp_gradient_sign_consistency: (0..n)
            .map(|i| sign_consistency(cp_pos_count[i], cp_neg_count[i]))
            .collect(),
        wdl_gradient_sign_consistency: (0..n)
            .map(|i| sign_consistency(wdl_pos_count[i], wdl_neg_count[i]))
            .collect(),
        cosine_similarity: (0..n)
            .map(|i| cosine_similarity(dot_sum[i], cp_sq_sum[i], wdl_sq_sum[i]))
            .collect(),
    }
}

/// Fraction of `flags` that are `true`.
pub fn ratio(flags: &[bool]) -> f32 {
    if flags.is_empty() {
        return 0.0;
    }
    flags.iter().filter(|&&b| b).count() as f32 / flags.len() as f32
}

/// Mean and (population) standard deviation from a running sum and
/// sum-of-squares, e.g. `Trainer::output_sum`/`output_sum_sq`.
pub fn mean_std(sum: f64, sum_sq: f64, n: u64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let n = n as f64;
    let mean = sum / n;
    // max(0.0) guards against a tiny negative from floating-point rounding
    // when the true variance is ~0 (e.g. output collapsed to a constant).
    let variance = (sum_sq / n - mean * mean).max(0.0);
    (mean, variance.sqrt())
}

/// Whole-parameter-vector L2 (Euclidean) distance between two
/// same-length snapshots from `TrainWeights::snapshot_params`.
pub fn l2_diff_norm(prev: &[f32], curr: &[f32]) -> f32 {
    debug_assert_eq!(prev.len(), curr.len());
    prev.iter()
        .zip(curr.iter())
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f32>()
        .sqrt()
}

/// Fraction of quantised FT weights (i16, post `to_nnue_weights`) that
/// rounded to exactly zero — a proxy for how much of the feature
/// transformer survived quantisation at all, distinct from
/// `ft_active_ratio` (forward-pass activation, not raw weight magnitude).
pub fn quantized_ft_zero_ratio(w: &NnueWeights) -> f32 {
    let total: usize = w.ft.iter().map(|row| row.len()).sum();
    if total == 0 {
        return 0.0;
    }
    let zeros = w.ft.iter().flatten().filter(|&&v| v == 0).count();
    zeros as f32 / total as f32
}

/// Per-neuron activation frequency: fraction of samples this epoch where
/// the L2 neuron's post-activation was > 0 (i.e. `1 - dead frequency`).
/// `zero_count[o]` counts samples where the pre-clamp value was <= 0.
pub fn l2_activation_frequency_per_neuron(zero_count: &[u64], sample_count: u64) -> Vec<f32> {
    if sample_count == 0 {
        return vec![0.0; zero_count.len()];
    }
    zero_count
        .iter()
        .map(|&z| 1.0 - z as f32 / sample_count as f32)
        .collect()
}

/// Per-neuron saturation frequency: fraction of samples this epoch where
/// the L2 neuron's pre-clamp value was >= 127 (the ClippedReLU ceiling).
/// `sat_count[o]` counts those samples.
pub fn l2_saturation_frequency_per_neuron(sat_count: &[u64], sample_count: u64) -> Vec<f32> {
    if sample_count == 0 {
        return vec![0.0; sat_count.len()];
    }
    sat_count
        .iter()
        .map(|&s| s as f32 / sample_count as f32)
        .collect()
}

/// Count of L2 neurons dead for *every* sample this epoch (post-activation
/// == 0 for all samples, i.e. activation frequency == 0.0).
pub fn l2_dead_neurons(zero_count: &[u64], sample_count: u64) -> usize {
    if sample_count == 0 {
        return 0;
    }
    zero_count.iter().filter(|&&z| z == sample_count).count()
}

/// Percentiles of `values` at each fraction in `qs` (each in `[0, 1]`),
/// nearest-rank on a full sort.
///
/// ponytail: collect-and-sort is O(n log n) over one epoch's L2
/// pre-activations -- fine at this dataset's scale; switch to a streaming
/// quantile sketch if per-epoch sample counts grow much larger.
pub fn percentiles(values: &[f32], qs: &[f32]) -> Vec<f32> {
    if values.is_empty() {
        return vec![0.0; qs.len()];
    }
    let mut sorted: Vec<f32> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    qs.iter()
        .map(|&q| {
            let idx = (q.clamp(0.0, 1.0) * (sorted.len() - 1) as f32).round() as usize;
            sorted[idx]
        })
        .collect()
}

/// Per-neuron incoming weight-row L2 (Euclidean) norm: for output neuron
/// `o`, the norm over the `rows`-length column `l2[.., o]` of a flat
/// `rows` × `cols` row-major matrix (matches `TrainWeights::l2`'s layout).
pub fn l2_row_weight_norm_per_neuron(l2: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    (0..cols)
        .map(|o| {
            (0..rows)
                .map(|i| {
                    let v = l2[i * cols + o];
                    v * v
                })
                .sum::<f32>()
                .sqrt()
        })
        .collect()
}

/// Output-layer weight vector's L2 (Euclidean) norm -- `out` is a single
/// `L2`-length vector (one output neuron), not a matrix, so this is just
/// the whole-vector norm, unlike `l2_row_weight_norm_per_neuron`'s
/// per-output-neuron breakdown of the wider L2 layer.
pub fn output_weight_norm(out: &[f32]) -> f32 {
    out.iter().map(|&x| x * x).sum::<f32>().sqrt()
}

/// Pearson correlation coefficient between two equal-length series
/// summarized as sufficient statistics (`n`, `Σx`, `Σx²`, `Σy`, `Σy²`,
/// `Σxy`) -- lets callers fold this incrementally across an epoch (one
/// running accumulator pair) instead of keeping every sample around.
/// Returns `0.0` for `n < 2` or a zero-variance series (undefined
/// correlation) rather than `NaN`, since this feeds a printed diagnostic
/// line and `.meta.json`, not further arithmetic.
#[allow(clippy::too_many_arguments)]
pub fn pearson_correlation(
    n: u64,
    sum_x: f64,
    sum_x2: f64,
    sum_y: f64,
    sum_y2: f64,
    sum_xy: f64,
) -> f64 {
    if n < 2 {
        return 0.0;
    }
    let n = n as f64;
    let cov = sum_xy - sum_x * sum_y / n;
    let var_x = sum_x2 - sum_x * sum_x / n;
    let var_y = sum_y2 - sum_y * sum_y / n;
    let denom = (var_x * var_y).max(0.0).sqrt();
    if denom <= 0.0 {
        return 0.0;
    }
    (cov / denom).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekirei_core::nnue::{L1, L2};

    #[test]
    fn ratio_of_empty_slice_is_zero() {
        assert_eq!(ratio(&[]), 0.0);
    }

    #[test]
    fn ratio_counts_true_fraction() {
        assert_eq!(ratio(&[true, false, true, true]), 0.75);
    }

    #[test]
    fn mean_std_zero_count_is_zero() {
        assert_eq!(mean_std(0.0, 0.0, 0), (0.0, 0.0));
    }

    #[test]
    fn mean_std_matches_hand_computed_values() {
        // xs = [1.0, 2.0, 3.0] -> mean=2.0, population variance=2/3
        let (sum, sum_sq) = [1.0f64, 2.0, 3.0]
            .iter()
            .fold((0.0, 0.0), |(s, sq), &x| (s + x, sq + x * x));
        let (mean, std) = mean_std(sum, sum_sq, 3);
        assert!((mean - 2.0).abs() < 1e-9);
        assert!((std - (2.0f64 / 3.0).sqrt()).abs() < 1e-9);
    }

    #[test]
    fn l2_diff_norm_zero_for_identical_snapshots() {
        let a = [1.0f32, 2.0, 3.0];
        assert_eq!(l2_diff_norm(&a, &a), 0.0);
    }

    #[test]
    fn l2_diff_norm_matches_hand_computed_euclidean_distance() {
        let a = [0.0f32, 0.0];
        let b = [3.0f32, 4.0];
        assert_eq!(l2_diff_norm(&a, &b), 5.0); // 3-4-5 triangle
    }

    #[test]
    fn quantized_ft_zero_ratio_counts_exact_zeros() {
        let mut w = NnueWeights {
            ft: vec![[1i16; L1]; 10], // 10 rows, all nonzero
            ft_bias: [0i16; L1],
            l2: vec![[0.0f32; L2]; 2 * L1],
            l2_bias: [0.0f32; L2],
            out: [0.0f32; L2],
            out_bias: 0.0,
        };
        assert_eq!(quantized_ft_zero_ratio(&w), 0.0);
        w.ft[0] = [0i16; L1]; // zero out one whole row
        let expected = L1 as f32 / (10 * L1) as f32;
        assert!((quantized_ft_zero_ratio(&w) - expected).abs() < 1e-6);
    }

    #[test]
    fn quantized_ft_zero_ratio_of_empty_ft_is_zero() {
        let w = NnueWeights {
            ft: vec![],
            ft_bias: [0i16; L1],
            l2: vec![[0.0f32; L2]; 2 * L1],
            l2_bias: [0.0f32; L2],
            out: [0.0f32; L2],
            out_bias: 0.0,
        };
        assert_eq!(quantized_ft_zero_ratio(&w), 0.0);
    }

    #[test]
    fn l2_activation_frequency_matches_hand_computed_values() {
        // sample_count=4: neuron0 dead 2/4 -> freq 0.5, neuron1 never dead
        // -> freq 1.0, neuron2 always dead -> freq 0.0
        let zero_count = [2u64, 0, 4];
        assert_eq!(
            l2_activation_frequency_per_neuron(&zero_count, 4),
            vec![0.5, 1.0, 0.0]
        );
    }

    #[test]
    fn l2_activation_frequency_zero_samples_is_zero_filled() {
        assert_eq!(
            l2_activation_frequency_per_neuron(&[1, 2], 0),
            vec![0.0, 0.0]
        );
    }

    #[test]
    fn l2_saturation_frequency_matches_hand_computed_values() {
        let sat_count = [1u64, 4, 0];
        assert_eq!(
            l2_saturation_frequency_per_neuron(&sat_count, 4),
            vec![0.25, 1.0, 0.0]
        );
    }

    #[test]
    fn l2_dead_neurons_counts_only_fully_dead() {
        // sample_count=4: neuron0 dead every sample, neuron2 dead every
        // sample, neuron1/3 fire at least once -> 2 dead
        let zero_count = [4u64, 3, 4, 0];
        assert_eq!(l2_dead_neurons(&zero_count, 4), 2);
    }

    #[test]
    fn l2_dead_neurons_zero_samples_is_zero() {
        assert_eq!(l2_dead_neurons(&[0, 0], 0), 0);
    }

    #[test]
    fn percentiles_matches_hand_computed_median_and_extremes() {
        let values = [5.0f32, 1.0, 3.0, 2.0, 4.0]; // sorted: 1,2,3,4,5
        assert_eq!(percentiles(&values, &[0.0, 0.5, 1.0]), vec![1.0, 3.0, 5.0]);
    }

    #[test]
    fn percentiles_of_empty_is_zero_filled() {
        assert_eq!(percentiles(&[], &[0.5, 0.9]), vec![0.0, 0.0]);
    }

    #[test]
    fn l2_row_weight_norm_matches_hand_computed_euclidean_distance() {
        // rows=2, cols=2, flat row-major: col0 = [3,4] (norm 5), col1 = [0,0]
        let l2 = [3.0f32, 0.0, 4.0, 0.0];
        assert_eq!(l2_row_weight_norm_per_neuron(&l2, 2, 2), vec![5.0, 0.0]);
    }

    #[test]
    fn output_weight_norm_matches_hand_computed_euclidean_norm() {
        assert_eq!(output_weight_norm(&[3.0, 4.0]), 5.0);
        assert_eq!(output_weight_norm(&[]), 0.0);
    }

    #[test]
    fn pearson_correlation_perfect_positive_line_is_one() {
        // y = 2x: x=[1,2,3], y=[2,4,6]
        let (n, mut sx, mut sx2, mut sy, mut sy2, mut sxy) = (3u64, 0.0, 0.0, 0.0, 0.0, 0.0);
        for (x, y) in [(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)] {
            sx += x;
            sx2 += x * x;
            sy += y;
            sy2 += y * y;
            sxy += x * y;
        }
        let r = pearson_correlation(n, sx, sx2, sy, sy2, sxy);
        assert!((r - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pearson_correlation_perfect_negative_line_is_minus_one() {
        let (n, mut sx, mut sx2, mut sy, mut sy2, mut sxy) = (3u64, 0.0, 0.0, 0.0, 0.0, 0.0);
        for (x, y) in [(1.0, 6.0), (2.0, 4.0), (3.0, 2.0)] {
            sx += x;
            sx2 += x * x;
            sy += y;
            sy2 += y * y;
            sxy += x * y;
        }
        let r = pearson_correlation(n, sx, sx2, sy, sy2, sxy);
        assert!((r - (-1.0)).abs() < 1e-9);
    }

    #[test]
    fn pearson_correlation_constant_series_is_zero_not_nan() {
        // y is constant -> zero variance -> undefined correlation, must
        // return 0.0 (not NaN) since this feeds a printed diagnostic line.
        let (n, sx, sx2, sy, sy2, sxy) = (3u64, 6.0, 14.0, 15.0, 75.0, 30.0);
        assert_eq!(pearson_correlation(n, sx, sx2, sy, sy2, sxy), 0.0);
    }

    #[test]
    fn pearson_correlation_fewer_than_two_samples_is_zero() {
        assert_eq!(pearson_correlation(0, 0.0, 0.0, 0.0, 0.0, 0.0), 0.0);
        assert_eq!(pearson_correlation(1, 5.0, 25.0, 5.0, 25.0, 25.0), 0.0);
    }
}
