/// Elo difference: positive means engine1 is stronger.
/// Formula: 400 * log10(score / (1 - score)) where score = (W + 0.5*D) / N
pub fn elo_diff(e1_wins: u32, draws: u32, e2_wins: u32) -> f64 {
    let n = e1_wins + draws + e2_wins;
    if n == 0 {
        return 0.0;
    }
    let score = ((e1_wins as f64) + 0.5 * (draws as f64)) / (n as f64);
    let score = score.clamp(0.001, 0.999);
    -400.0 * (1.0 / score - 1.0).log10()
}

/// 95% confidence interval half-width in Elo units.
/// Propagates SE of score (normal approx) through the Elo derivative.
pub fn elo_ci(e1_wins: u32, draws: u32, e2_wins: u32) -> f64 {
    let n = (e1_wins + draws + e2_wins) as f64;
    if n < 2.0 {
        return f64::INFINITY;
    }
    let score = ((e1_wins as f64) + 0.5 * (draws as f64)) / n;
    let score = score.clamp(0.001, 0.999);
    let se_score = (score * (1.0 - score) / n).sqrt();
    let derivative = 400.0 / (std::f64::consts::LN_10 * score * (1.0 - score));
    1.96 * derivative * se_score
}

/// Likelihood of superiority: P(engine1 > engine2).
/// Uses normal CDF approximation: LOS = Φ(Elo_diff / SE_elo)
pub fn los(e1_wins: u32, draws: u32, e2_wins: u32) -> f64 {
    let n = (e1_wins + draws + e2_wins) as f64;
    if n < 2.0 {
        return 0.5;
    }
    let score = ((e1_wins as f64) + 0.5 * (draws as f64)) / n;
    let score = score.clamp(0.001, 0.999);
    let se_score = (score * (1.0 - score) / n).sqrt();
    // z = (score - 0.5) / se_score
    let z = (score - 0.5) / se_score;
    normal_cdf(z)
}

fn normal_cdf(z: f64) -> f64 {
    0.5 * (1.0 + erf(z / std::f64::consts::SQRT_2))
}

// Abramowitz & Stegun rational approximation, max error 1.5e-7
fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    sign * y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elo_50pct_is_zero() {
        assert!(elo_diff(5, 0, 5).abs() < 0.01);
    }

    #[test]
    fn elo_75pct_is_about_191() {
        let d = elo_diff(75, 0, 25);
        assert!((d - 190.85).abs() < 1.0, "got {d}");
    }

    #[test]
    fn elo_ci_decreases_with_more_games() {
        let ci10 = elo_ci(6, 0, 4);
        let ci100 = elo_ci(60, 0, 40);
        assert!(ci100 < ci10);
    }

    #[test]
    fn los_50pct_is_half() {
        let l = los(5, 0, 5);
        assert!((l - 0.5).abs() < 0.02);
    }

    #[test]
    fn los_all_wins_is_high() {
        let l = los(20, 0, 0);
        assert!(l > 0.99);
    }
}
