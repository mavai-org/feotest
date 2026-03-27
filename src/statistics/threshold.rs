//! Threshold derivation from baseline data.
//!
//! Derives pass/fail thresholds using either:
//! - the Wilson one-sided lower bound from baseline observations
//!   ([`derive_sample_size_first`]), or
//! - binary search for the implied confidence given an explicit threshold
//!   ([`derive_threshold_first`]).

use crate::statistics::defaults::SOUNDNESS_FLOOR;
use crate::statistics::proportion;
use crate::statistics::types::{
    ConfidenceLevel, DerivationContext, DerivedThreshold, OperationalApproach,
};

/// Derives a threshold from the Wilson one-sided lower bound of the baseline.
///
/// This is the **sample-size-first** approach: given a fixed number of
/// baseline and test trials and a desired confidence level, the threshold
/// is set to the lowest plausible baseline success rate.
///
/// # Panics
///
/// Panics if `test_samples` is zero, `baseline_samples` is zero, or
/// `baseline_successes > baseline_samples`.
#[must_use]
pub fn derive_sample_size_first(
    baseline_successes: u32,
    baseline_samples: u32,
    test_samples: u32,
    confidence: ConfidenceLevel,
) -> DerivedThreshold {
    assert!(test_samples > 0, "test_samples must be positive");

    let threshold_value = proportion::lower_bound(baseline_successes, baseline_samples, confidence);
    let baseline_rate = f64::from(baseline_successes) / f64::from(baseline_samples);

    let context = DerivationContext::new(baseline_rate, baseline_samples, test_samples, confidence);
    DerivedThreshold::new(
        threshold_value,
        OperationalApproach::SampleSizeFirst,
        context,
        true,
    )
}

/// Derives the implied confidence for a given explicit threshold.
///
/// This is the **threshold-first** approach: given baseline data and a
/// desired threshold, find the confidence level at which the Wilson
/// one-sided lower bound equals that threshold.
///
/// Uses binary search over the confidence level. The result is flagged
/// as statistically unsound if the implied confidence is below 80%.
///
/// # Panics
///
/// Panics if `test_samples` or `baseline_samples` is zero,
/// `baseline_successes > baseline_samples`, or `explicit_threshold` is
/// not in [0, 1].
#[must_use]
pub fn derive_threshold_first(
    baseline_successes: u32,
    baseline_samples: u32,
    test_samples: u32,
    explicit_threshold: f64,
) -> DerivedThreshold {
    assert!(test_samples > 0, "test_samples must be positive");
    assert!(
        (0.0..=1.0).contains(&explicit_threshold),
        "explicit_threshold must be in [0, 1], got {explicit_threshold}"
    );

    let baseline_rate = f64::from(baseline_successes) / f64::from(baseline_samples);

    let implied_confidence =
        find_implied_confidence(baseline_successes, baseline_samples, explicit_threshold);

    let confidence = ConfidenceLevel::new(implied_confidence);
    let is_sound = implied_confidence >= SOUNDNESS_FLOOR;

    let context = DerivationContext::new(baseline_rate, baseline_samples, test_samples, confidence);
    DerivedThreshold::new(
        explicit_threshold,
        OperationalApproach::ThresholdFirst,
        context,
        is_sound,
    )
}

/// Binary search for the confidence level at which the Wilson one-sided
/// lower bound equals the target threshold.
fn find_implied_confidence(successes: u32, trials: u32, target: f64) -> f64 {
    let mut lo = 1e-6_f64;
    let mut hi = 1.0 - 1e-6;

    // At very low confidence the lower bound should be above the target;
    // at very high confidence it should be below. We want the crossing point.
    for _ in 0..100 {
        let mid = f64::midpoint(lo, hi);
        let cl = ConfidenceLevel::new(mid);
        let lb = proportion::lower_bound(successes, trials, cl);

        if lb > target {
            // Lower bound is still above target → need higher confidence
            // (which pushes the lower bound down).
            lo = mid;
        } else {
            hi = mid;
        }
    }

    f64::midpoint(lo, hi)
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn cl(v: f64) -> ConfidenceLevel {
        ConfidenceLevel::new(v)
    }

    // --- derive_sample_size_first ---

    #[test]
    fn threshold_is_below_baseline_rate() {
        let dt = derive_sample_size_first(90, 100, 100, cl(0.95));
        assert!(dt.value() < 0.9);
        assert!(dt.value() > 0.0);
    }

    #[test]
    fn threshold_approach_is_sample_size_first() {
        let dt = derive_sample_size_first(90, 100, 100, cl(0.95));
        assert_eq!(dt.approach(), OperationalApproach::SampleSizeFirst);
    }

    #[test]
    fn threshold_is_always_statistically_sound_for_sample_size_first() {
        let dt = derive_sample_size_first(90, 100, 100, cl(0.95));
        assert!(dt.is_statistically_sound());
    }

    #[test]
    fn threshold_gap_is_positive() {
        let dt = derive_sample_size_first(90, 100, 100, cl(0.95));
        assert!(dt.gap_from_baseline() > 0.0);
    }

    #[test]
    fn higher_confidence_produces_lower_threshold() {
        let t90 = derive_sample_size_first(90, 100, 100, cl(0.90));
        let t99 = derive_sample_size_first(90, 100, 100, cl(0.99));
        assert!(t99.value() < t90.value());
    }

    #[test]
    fn more_baseline_samples_produces_tighter_threshold() {
        let small = derive_sample_size_first(9, 10, 100, cl(0.95));
        let large = derive_sample_size_first(900, 1000, 100, cl(0.95));
        // Both have p̂ = 0.9, but more data → less uncertainty → higher threshold
        assert!(large.value() > small.value());
    }

    #[test]
    #[should_panic(expected = "test_samples must be positive")]
    fn panics_on_zero_test_samples() {
        derive_sample_size_first(90, 100, 0, cl(0.95));
    }

    #[test]
    #[should_panic(expected = "trials must be positive")]
    fn panics_on_zero_baseline_trials() {
        derive_sample_size_first(0, 0, 100, cl(0.95));
    }

    // --- derive_threshold_first ---

    #[test]
    fn threshold_first_returns_explicit_threshold_value() {
        let dt = derive_threshold_first(90, 100, 100, 0.85);
        assert_relative_eq!(dt.value(), 0.85, epsilon = 1e-10);
    }

    #[test]
    fn threshold_first_approach_is_threshold_first() {
        let dt = derive_threshold_first(90, 100, 100, 0.85);
        assert_eq!(dt.approach(), OperationalApproach::ThresholdFirst);
    }

    #[test]
    fn threshold_first_high_threshold_is_unsound() {
        // Threshold close to baseline rate → low confidence → unsound
        let dt = derive_threshold_first(90, 100, 100, 0.89);
        assert!(!dt.is_statistically_sound());
    }

    #[test]
    fn threshold_first_low_threshold_is_sound() {
        // Threshold well below baseline → high implied confidence → sound
        let dt = derive_threshold_first(90, 100, 100, 0.70);
        assert!(dt.is_statistically_sound());
    }

    #[test]
    #[should_panic(expected = "explicit_threshold must be in")]
    fn threshold_first_panics_on_invalid_threshold_above() {
        derive_threshold_first(90, 100, 100, 1.5);
    }

    #[test]
    #[should_panic(expected = "explicit_threshold must be in")]
    fn threshold_first_panics_on_invalid_threshold_below() {
        derive_threshold_first(90, 100, 100, -0.1);
    }

    #[test]
    #[should_panic(expected = "test_samples must be positive")]
    fn threshold_first_panics_on_zero_test_samples() {
        derive_threshold_first(90, 100, 0, 0.85);
    }

    // --- round-trip: derive then check implied confidence ---

    #[test]
    fn round_trip_sample_size_first_then_threshold_first() {
        // Derive threshold at 95% confidence, then recover the confidence
        let dt1 = derive_sample_size_first(90, 100, 100, cl(0.95));
        let dt2 = derive_threshold_first(90, 100, 100, dt1.value());
        // The recovered confidence should be close to 0.95
        assert_relative_eq!(dt2.context().confidence().value(), 0.95, epsilon = 1e-3);
    }
}
