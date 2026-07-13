//! Per-epoch training diagnostics — pure functions over counters the
//! trainer accumulates during an epoch, or over saved weights.
//!
//! The only prior "diagnostic" this project had was a one-off manual read
//! of saved weight files that found the 2026-07-09 capacity-collapse bug
//! (every FT/L2 row a single repeated scalar). These functions turn that
//! kind of check into a routine per-epoch printout instead of something
//! only found by accident, months after the fact.

use sekirei_core::nnue::NnueWeights;

#[derive(Debug, Clone, Copy)]
pub struct EpochDiagnostics {
    /// Whole-parameter-vector L2 distance from the previous epoch's
    /// snapshot. `None` on the first epoch (no previous snapshot exists).
    pub param_update_norm: Option<f32>,
    pub ft_active_ratio: f32,
    pub l2_active_ratio: f32,
    pub ft_saturation_ratio: f32,
    pub l2_saturation_ratio: f32,
    pub output_mean: f64,
    pub output_std: f64,
    pub quantized_ft_zero_ratio: f32,
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
}
