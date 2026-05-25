//! Sample size calculation via power analysis.
//!
//! Implements the confidence-first operational approach: given a baseline rate,
//! a minimum detectable effect, confidence, and power, compute the required
//! sample size. Also provides the inverse: given a sample size, compute the
//! achieved power.

use statrs::distribution::{ContinuousCDF, Normal};

use crate::statistics::proportion;
use crate::statistics::types::{ConfidenceLevel, SampleSizeRequirement};

/// Returns the standard normal distribution N(0, 1).
fn standard_normal() -> Normal {
    Normal::new(0.0, 1.0).unwrap()
}

/// Computes the required sample size for a given power level.
///
/// Uses the formula:
///
/// n = ⌈((`z_α` × σ₀ + `z_β` × σ₁) / δ)²⌉
///
/// where:
/// - `z_α` = one-sided z-score at the confidence level
/// - `z_β` = one-sided z-score at the power level
/// - σ₀ = √(p₀ × (1 − p₀))  (std dev under null)
/// - σ₁ = √(p₁ × (1 − p₁))  (std dev under alternative, p₁ = p₀ − δ)
/// - δ = minimum detectable effect
///
/// # Panics
///
/// Panics if `baseline_rate` is not in [0, 1], `min_detectable_effect` is
/// not positive or exceeds `baseline_rate`, or `power` is not in (0, 1).
#[must_use]
// javai-ref: JVI-EGMJ0MU — do not remove (resolves in javai-orchestrator)
pub fn calculate_for_power(
    baseline_rate: f64,
    min_detectable_effect: f64,
    confidence: ConfidenceLevel,
    power: f64,
) -> SampleSizeRequirement {
    assert_valid_inputs(baseline_rate, min_detectable_effect, power);

    let p0 = baseline_rate;
    let p1 = p0 - min_detectable_effect;

    let z_alpha = proportion::z_score_one_sided(confidence);
    let power_cl = ConfidenceLevel::new(power);
    let z_beta = proportion::z_score_one_sided(power_cl);

    let sigma_0 = (p0 * (1.0 - p0)).sqrt();
    let sigma_1 = (p1 * (1.0 - p1)).sqrt();

    let numerator = z_alpha.mul_add(sigma_0, z_beta * sigma_1);
    let n_raw = (numerator / min_detectable_effect).powi(2);

    // Round up: we need at least this many samples.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "ceil-rounded sample count bounded by the problem scale"
    )]
    let required = n_raw.ceil() as u32;

    SampleSizeRequirement::new(required, confidence, power, min_detectable_effect, p0, p1)
}

/// Computes the achieved power for a given sample size.
///
/// This is the inverse of [`calculate_for_power`]: given all other
/// parameters, what power does the specified sample size deliver?
///
/// Power = `Φ(z_β)` where `z_β` = (δ × √n − `z_α` × σ₀) / σ₁
///
/// # Panics
///
/// Panics if `sample_size` is zero, `baseline_rate` is not in [0, 1],
/// or `min_detectable_effect` is not positive or exceeds `baseline_rate`.
#[must_use]
pub fn calculate_achieved_power(
    sample_size: u32,
    baseline_rate: f64,
    min_detectable_effect: f64,
    confidence: ConfidenceLevel,
) -> f64 {
    assert!(sample_size > 0, "sample_size must be positive");
    assert_valid_inputs(baseline_rate, min_detectable_effect, 0.5);

    let p0 = baseline_rate;
    let p1 = p0 - min_detectable_effect;

    let z_alpha = proportion::z_score_one_sided(confidence);

    let sigma_0 = (p0 * (1.0 - p0)).sqrt();
    let sigma_1 = (p1 * (1.0 - p1)).sqrt();

    if sigma_1 == 0.0 {
        // Alternative rate is 0 or 1 — power is trivially 1
        return 1.0;
    }

    let n = f64::from(sample_size);
    let z_beta = min_detectable_effect.mul_add(n.sqrt(), -(z_alpha * sigma_0)) / sigma_1;

    standard_normal().cdf(z_beta)
}

/// Asserts common preconditions for sample size calculations.
fn assert_valid_inputs(baseline_rate: f64, min_detectable_effect: f64, power: f64) {
    assert!(
        (0.0..=1.0).contains(&baseline_rate),
        "baseline_rate must be in [0, 1], got {baseline_rate}"
    );
    assert!(
        min_detectable_effect > 0.0 && min_detectable_effect < baseline_rate,
        "min_detectable_effect ({min_detectable_effect}) must be in (0, baseline_rate={baseline_rate})"
    );
    assert!(
        power > 0.0 && power < 1.0,
        "power must be in (0, 1), got {power}"
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

    // --- calculate_for_power ---

    #[test]
    fn reasonable_sample_size_for_standard_params() {
        // p0=0.9, delta=0.05, confidence=0.95, power=0.80
        let req = calculate_for_power(0.9, 0.05, cl(0.95), 0.80);
        assert!(req.required_samples() > 50);
        assert!(req.required_samples() < 1000);
    }

    #[test]
    fn smaller_effect_requires_more_samples() {
        let large_effect = calculate_for_power(0.9, 0.10, cl(0.95), 0.80);
        let small_effect = calculate_for_power(0.9, 0.05, cl(0.95), 0.80);
        assert!(small_effect.required_samples() > large_effect.required_samples());
    }

    #[test]
    fn higher_power_requires_more_samples() {
        let low_power = calculate_for_power(0.9, 0.05, cl(0.95), 0.80);
        let high_power = calculate_for_power(0.9, 0.05, cl(0.95), 0.95);
        assert!(high_power.required_samples() > low_power.required_samples());
    }

    #[test]
    fn higher_confidence_requires_more_samples() {
        let low_conf = calculate_for_power(0.9, 0.05, cl(0.90), 0.80);
        let high_conf = calculate_for_power(0.9, 0.05, cl(0.99), 0.80);
        assert!(high_conf.required_samples() > low_conf.required_samples());
    }

    #[test]
    fn requirement_records_all_parameters() {
        let req = calculate_for_power(0.9, 0.05, cl(0.95), 0.80);
        assert_relative_eq!(req.null_rate(), 0.9, epsilon = 1e-10);
        assert_relative_eq!(req.alternative_rate(), 0.85, epsilon = 1e-10);
        assert_relative_eq!(req.min_detectable_effect(), 0.05, epsilon = 1e-10);
        assert_relative_eq!(req.power(), 0.80, epsilon = 1e-10);
        assert_relative_eq!(req.confidence().value(), 0.95, epsilon = 1e-10);
    }

    #[test]
    #[should_panic(expected = "baseline_rate must be in")]
    fn panics_on_invalid_baseline_rate() {
        calculate_for_power(1.5, 0.05, cl(0.95), 0.80);
    }

    #[test]
    #[should_panic(expected = "min_detectable_effect")]
    fn panics_on_effect_exceeding_baseline() {
        calculate_for_power(0.9, 0.95, cl(0.95), 0.80);
    }

    #[test]
    #[should_panic(expected = "min_detectable_effect")]
    fn panics_on_zero_effect() {
        calculate_for_power(0.9, 0.0, cl(0.95), 0.80);
    }

    #[test]
    #[should_panic(expected = "power must be in")]
    fn panics_on_invalid_power() {
        calculate_for_power(0.9, 0.05, cl(0.95), 0.0);
    }

    // --- calculate_achieved_power ---

    #[test]
    fn achieved_power_meets_target_at_calculated_size() {
        let req = calculate_for_power(0.9, 0.05, cl(0.95), 0.80);
        let achieved = calculate_achieved_power(req.required_samples(), 0.9, 0.05, cl(0.95));
        // Should meet or exceed the target power (we round up n)
        assert!(achieved >= 0.80 - 1e-3);
    }

    #[test]
    fn achieved_power_increases_with_sample_size() {
        let p1 = calculate_achieved_power(100, 0.9, 0.05, cl(0.95));
        let p2 = calculate_achieved_power(500, 0.9, 0.05, cl(0.95));
        assert!(p2 > p1);
    }

    #[test]
    #[should_panic(expected = "sample_size must be positive")]
    fn panics_on_zero_sample_size() {
        calculate_achieved_power(0, 0.9, 0.05, cl(0.95));
    }

    // --- round-trip: calculate n then verify power ---

    #[test]
    fn round_trip_power_calculation() {
        let params = [(0.9, 0.05), (0.8, 0.10), (0.95, 0.03)];
        for (p0, delta) in params {
            let req = calculate_for_power(p0, delta, cl(0.95), 0.80);
            let achieved = calculate_achieved_power(req.required_samples(), p0, delta, cl(0.95));
            assert!(
                achieved >= 0.80 - 1e-3,
                "Round-trip failed for p0={p0}, delta={delta}: achieved={achieved}"
            );
        }
    }
}
