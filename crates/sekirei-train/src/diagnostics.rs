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
