//! Binomial proportion estimation using Wilson score intervals.
//!
//! All confidence intervals use the Wilson score method, which is well-behaved
//! near p̂ = 0 and p̂ = 1 and never produces bounds outside [0, 1].

use statrs::distribution::{ContinuousCDF, Normal};

use crate::statistics::types::{ConfidenceLevel, ProportionEstimate};

/// Returns the standard normal distribution N(0, 1).
///
/// # Panics
///
/// Cannot panic — parameters are compile-time constants.
fn standard_normal() -> Normal {
    Normal::new(0.0, 1.0).unwrap()
}

/// Computes the standard error of a sample proportion.
///
/// SE(p̂) = √(p̂ × (1 − p̂) / n)
///
/// Returns 0 when p̂ is 0 or 1 (no variability).
///
/// # Panics
///
/// Panics if `trials` is zero or `successes > trials`.
#[must_use]
pub fn standard_error(successes: u32, trials: u32) -> f64 {
    assert_valid_successes_trials(successes, trials);
    let n = f64::from(trials);
    let p_hat = f64::from(successes) / n;
    (p_hat * (1.0 - p_hat) / n).sqrt()
}

/// Computes a two-sided Wilson score confidence interval.
///
/// Returns a [`ProportionEstimate`] containing the point estimate p̂,
/// the Wilson lower and upper bounds, and the confidence level used.
///
/// # Panics
///
/// Panics if `trials` is zero or `successes > trials`.
#[must_use]
pub fn estimate(successes: u32, trials: u32, confidence: ConfidenceLevel) -> ProportionEstimate {
    assert_valid_successes_trials(successes, trials);

    let n = f64::from(trials);
    let p_hat = f64::from(successes) / n;
    let z = z_score_two_sided(confidence);
    let z2 = z * z;

    let denominator = 1.0 + z2 / n;
    let center = (p_hat + z2 / (2.0 * n)) / denominator;
    let margin = z * ((p_hat * (1.0 - p_hat) / n) + (z2 / (4.0 * n * n))).sqrt() / denominator;

    let lower = center - margin;
    let upper = center + margin;

    ProportionEstimate::new(p_hat, trials, lower, upper, confidence)
}

/// Computes the one-sided Wilson lower bound from discrete counts.
///
/// This is the critical function for threshold derivation: it gives the
/// lowest plausible success rate at the given confidence level. Uses
/// `z_α` (one-sided) rather than z_{α/2} (two-sided).
///
/// # Panics
///
/// Panics if `trials` is zero or `successes > trials`.
// javai-ref: JVI-MNVWS4U — do not remove (resolves in javai-orchestrator)
#[must_use]
pub fn lower_bound(successes: u32, trials: u32, confidence: ConfidenceLevel) -> f64 {
    assert_valid_successes_trials(successes, trials);
    let p_hat = f64::from(successes) / f64::from(trials);
    lower_bound_from_rate(p_hat, trials, confidence)
}

/// Computes the one-sided Wilson lower bound from a continuous rate.
///
/// Same Wilson formula as [`lower_bound`], but takes a continuous
/// proportion `p_hat` rather than discrete successes. Used by the
/// two-step threshold construction (statistical companion §4.3.2),
/// where the second step needs to apply Wilson at `n_test` with a
/// rate already derived from the baseline.
///
/// # Panics
///
/// Panics if `trials` is zero or `p_hat` is not in `[0, 1]`.
#[must_use]
pub fn lower_bound_from_rate(p_hat: f64, trials: u32, confidence: ConfidenceLevel) -> f64 {
    assert!(trials > 0, "trials must be positive");
    assert!(
        (0.0..=1.0).contains(&p_hat),
        "p_hat must be in [0, 1], got {p_hat}"
    );

    let n = f64::from(trials);
    let z = z_score_one_sided(confidence);
    let z2 = z * z;

    let denominator = 1.0 + z2 / n;
    let center = (p_hat + z2 / (2.0 * n)) / denominator;
    let margin = z * ((p_hat * (1.0 - p_hat) / n) + (z2 / (4.0 * n * n))).sqrt() / denominator;

    (center - margin).clamp(0.0, 1.0)
}

/// Returns the one-sided z-score for the given confidence level.
///
/// z = Φ⁻¹(1 − α) where α = 1 − confidence.
#[must_use]
pub fn z_score_one_sided(confidence: ConfidenceLevel) -> f64 {
    standard_normal().inverse_cdf(confidence.value())
}

/// Returns the two-sided z-score for the given confidence level.
///
/// z = Φ⁻¹(1 − α/2) where α = 1 − confidence.
#[must_use]
pub fn z_score_two_sided(confidence: ConfidenceLevel) -> f64 {
    let alpha = confidence.alpha();
    standard_normal().inverse_cdf(1.0 - alpha / 2.0)
}

/// Computes the one-sided z-test statistic.
///
/// z = (p̂ − π₀) / √(π₀ × (1 − π₀) / n)
///
/// # Panics
///
/// Panics if `observed_rate` or `hypothesized_rate` is not in [0, 1],
/// or if `sample_size` is zero.
#[must_use]
pub fn z_test_statistic(observed_rate: f64, hypothesized_rate: f64, sample_size: u32) -> f64 {
    assert!(
        (0.0..=1.0).contains(&observed_rate),
        "observed_rate must be in [0, 1], got {observed_rate}"
    );
    assert!(
        (0.0..=1.0).contains(&hypothesized_rate),
        "hypothesized_rate must be in [0, 1], got {hypothesized_rate}"
    );
    assert!(sample_size > 0, "sample_size must be positive");

    let n = f64::from(sample_size);
    let se = (hypothesized_rate * (1.0 - hypothesized_rate) / n).sqrt();

    if se == 0.0 {
        // Hypothesized rate is 0 or 1; z is undefined but effectively ±∞.
        return if observed_rate >= hypothesized_rate {
            0.0
        } else {
            f64::NEG_INFINITY
        };
    }

    (observed_rate - hypothesized_rate) / se
}

/// Returns the one-sided p-value (lower tail) for the given z-score.
///
/// P(Z ≤ z) — the probability of observing a value at least as extreme
/// under the null hypothesis, for a left-tailed test.
#[must_use]
pub fn one_sided_p_value(z: f64) -> f64 {
    standard_normal().cdf(z)
}

/// Asserts that trials > 0 and successes ≤ trials.
fn assert_valid_successes_trials(successes: u32, trials: u32) {
    assert!(trials > 0, "trials must be positive");
    assert!(
        successes <= trials,
        "successes ({successes}) cannot exceed trials ({trials})"
    );
}

#[cfg(test)]
#[allow(unused_must_use, reason = "test boilerplate may drop must_use values")]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn cl(v: f64) -> ConfidenceLevel {
        ConfidenceLevel::new(v)
    }

    // --- z-score tests against standard normal tables ---

    #[test]
    fn z_score_one_sided_at_95_percent() {
        assert_relative_eq!(z_score_one_sided(cl(0.95)), 1.6449, epsilon = 1e-4);
    }

    #[test]
    fn z_score_two_sided_at_95_percent() {
        assert_relative_eq!(z_score_two_sided(cl(0.95)), 1.9600, epsilon = 1e-4);
    }

    #[test]
    fn z_score_one_sided_at_99_percent() {
        assert_relative_eq!(z_score_one_sided(cl(0.99)), 2.3263, epsilon = 1e-4);
    }

    #[test]
    fn z_score_two_sided_at_99_percent() {
        assert_relative_eq!(z_score_two_sided(cl(0.99)), 2.5758, epsilon = 1e-4);
    }

    #[test]
    fn z_score_one_sided_at_90_percent() {
        assert_relative_eq!(z_score_one_sided(cl(0.90)), 1.2816, epsilon = 1e-4);
    }

    // --- standard error tests ---

    #[test]
    fn standard_error_of_fair_coin() {
        // 50/100, SE = sqrt(0.5 * 0.5 / 100) = 0.05
        assert_relative_eq!(standard_error(50, 100), 0.05, epsilon = 1e-10);
    }

    #[test]
    fn standard_error_collapses_at_zero() {
        assert_relative_eq!(standard_error(0, 100), 0.0, epsilon = 1e-10);
    }

    #[test]
    fn standard_error_collapses_at_one() {
        assert_relative_eq!(standard_error(100, 100), 0.0, epsilon = 1e-10);
    }

    #[test]
    #[should_panic(expected = "trials must be positive")]
    fn standard_error_panics_on_zero_trials() {
        standard_error(0, 0);
    }

    #[test]
    #[should_panic(expected = "cannot exceed trials")]
    fn standard_error_panics_when_successes_exceed_trials() {
        standard_error(10, 5);
    }

    // --- Wilson score CI tests ---

    #[test]
    fn wilson_ci_for_50_of_100_at_95_percent() {
        // Wilson score CI for p̂ = 0.5, n = 100, 95%:
        // lower ≈ 0.4038, upper ≈ 0.5962 (shifted toward 0.5 vs Wald)
        let est = estimate(50, 100, cl(0.95));
        assert_relative_eq!(est.point_estimate(), 0.5, epsilon = 1e-10);
        assert_relative_eq!(est.lower_bound(), 0.4038, epsilon = 1e-3);
        assert_relative_eq!(est.upper_bound(), 0.5962, epsilon = 1e-3);
        assert_eq!(est.sample_size(), 100);
    }

    #[test]
    fn wilson_ci_is_symmetric_for_fair_proportion() {
        let est = estimate(50, 100, cl(0.95));
        let mid = f64::midpoint(est.lower_bound(), est.upper_bound());
        // Wilson score center is slightly shifted from p̂ toward 0.5,
        // but for p̂ = 0.5 it should be centered.
        assert_relative_eq!(mid, est.point_estimate(), epsilon = 1e-3);
    }

    #[test]
    fn wilson_ci_contains_true_proportion_at_zero() {
        // p̂ = 0, n = 20 — lower bound should be 0, upper > 0
        let est = estimate(0, 20, cl(0.95));
        assert_relative_eq!(est.point_estimate(), 0.0, epsilon = 1e-10);
        assert_relative_eq!(est.lower_bound(), 0.0, epsilon = 1e-10);
        assert!(est.upper_bound() > 0.0);
    }

    #[test]
    fn wilson_ci_contains_true_proportion_at_one() {
        // p̂ = 1, n = 20 — upper bound should be 1, lower < 1
        let est = estimate(20, 20, cl(0.95));
        assert_relative_eq!(est.point_estimate(), 1.0, epsilon = 1e-10);
        assert_relative_eq!(est.upper_bound(), 1.0, epsilon = 1e-10);
        assert!(est.lower_bound() < 1.0);
    }

    #[test]
    fn wilson_ci_narrows_with_larger_sample() {
        let small = estimate(5, 10, cl(0.95));
        let large = estimate(500, 1000, cl(0.95));
        assert!(large.interval_width() < small.interval_width());
    }

    #[test]
    fn wilson_ci_widens_with_higher_confidence() {
        let low = estimate(50, 100, cl(0.90));
        let high = estimate(50, 100, cl(0.99));
        assert!(high.interval_width() > low.interval_width());
    }

    #[test]
    #[should_panic(expected = "trials must be positive")]
    fn estimate_panics_on_zero_trials() {
        estimate(0, 0, cl(0.95));
    }

    // --- one-sided lower bound tests ---

    #[test]
    fn lower_bound_below_point_estimate() {
        let lb = lower_bound(90, 100, cl(0.95));
        assert!(lb < 0.9);
        assert!(lb > 0.0);
    }

    #[test]
    fn lower_bound_for_perfect_baseline() {
        // p̂ = 1.0, n = 100 — lower bound should be close to 1 but < 1
        let lb = lower_bound(100, 100, cl(0.95));
        assert!(lb < 1.0);
        assert!(lb > 0.95);
    }

    #[test]
    fn lower_bound_increases_with_more_samples() {
        let small = lower_bound(9, 10, cl(0.95));
        let large = lower_bound(900, 1000, cl(0.95));
        assert!(large > small);
    }

    // --- z-test statistic tests ---

    #[test]
    fn z_test_observed_equals_hypothesized() {
        assert_relative_eq!(z_test_statistic(0.5, 0.5, 100), 0.0, epsilon = 1e-10);
    }

    #[test]
    fn z_test_observed_below_hypothesized() {
        assert!(z_test_statistic(0.4, 0.5, 100) < 0.0);
    }

    #[test]
    #[should_panic(expected = "observed_rate must be in")]
    fn z_test_panics_on_invalid_observed_rate() {
        z_test_statistic(1.5, 0.5, 100);
    }

    #[test]
    #[should_panic(expected = "hypothesized_rate must be in")]
    fn z_test_panics_on_invalid_hypothesized_rate() {
        z_test_statistic(0.5, -0.1, 100);
    }

    #[test]
    #[should_panic(expected = "sample_size must be positive")]
    fn z_test_panics_on_zero_sample_size() {
        z_test_statistic(0.5, 0.5, 0);
    }

    // --- p-value tests ---

    #[test]
    fn p_value_at_zero_is_half() {
        assert_relative_eq!(one_sided_p_value(0.0), 0.5, epsilon = 1e-10);
    }

    #[test]
    fn p_value_for_large_negative_z_is_near_zero() {
        assert!(one_sided_p_value(-4.0) < 0.001);
    }

    #[test]
    fn p_value_for_large_positive_z_is_near_one() {
        assert!(one_sided_p_value(4.0) > 0.999);
    }

    #[test]
    fn p_value_at_minus_1_96() {
        // P(Z ≤ -1.96) ≈ 0.025
        assert_relative_eq!(one_sided_p_value(-1.96), 0.025, epsilon = 1e-3);
    }

    // --- margin_of_error / interval_width ---

    #[test]
    fn margin_of_error_is_half_interval_width() {
        let est = estimate(50, 100, cl(0.95));
        assert_relative_eq!(
            est.margin_of_error(),
            est.interval_width() / 2.0,
            epsilon = 1e-10
        );
    }
}
