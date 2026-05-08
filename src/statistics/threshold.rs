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

/// Derives a threshold by applying the Wilson one-sided lower bound at
/// the *test* sample size to an effective baseline rate.
///
/// This is the **sample-size-first** approach (statistical companion §3.4
/// for the general case, §4.3.2 for the perfect-baseline two-step):
///
/// 1. Determine the effective baseline rate.
///    - If the baseline observed perfect success (`k = n`), the raw
///      rate of 1.0 has zero variance and would force the threshold to
///      1.0; instead, take the discrete one-sided Wilson lower bound at
///      `n_baseline` as the rate to carry forward (companion §4.3.2).
///    - Otherwise, the effective rate is simply the observed
///      proportion `k / n_baseline`.
/// 2. Apply the one-sided Wilson lower bound to that rate at the test
///    sample size — smaller test samples produce wider intervals and
///    therefore a lower threshold.
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

    let baseline_rate = f64::from(baseline_successes) / f64::from(baseline_samples);
    let effective_rate = if baseline_successes == baseline_samples {
        proportion::lower_bound(baseline_successes, baseline_samples, confidence)
    } else {
        baseline_rate
    };
    let threshold_value =
        proportion::lower_bound_from_rate(effective_rate, test_samples, confidence);

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
/// This is the **threshold-first** approach (statistical companion §6.3):
/// given baseline data, a test sample size, and a desired threshold,
/// find the confidence level at which the methodology used by
/// [`derive_sample_size_first`] would produce that threshold.
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

    let implied_confidence = find_implied_confidence(
        baseline_successes,
        baseline_samples,
        test_samples,
        explicit_threshold,
    );

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

/// Binary search for the confidence level at which
/// [`derive_sample_size_first`] would produce the target threshold.
fn find_implied_confidence(
    baseline_successes: u32,
    baseline_samples: u32,
    test_samples: u32,
    target: f64,
) -> f64 {
    let baseline_rate = f64::from(baseline_successes) / f64::from(baseline_samples);
    let perfect_baseline = baseline_successes == baseline_samples;

    let mut lo = 1e-6_f64;
    let mut hi = 1.0 - 1e-6;

    // At very low confidence the derived threshold should be above the
    // target; at very high confidence it should be below. We want the
    // crossing point.
    for _ in 0..100 {
        let mid = f64::midpoint(lo, hi);
        let cl = ConfidenceLevel::new(mid);
        let effective_rate = if perfect_baseline {
            proportion::lower_bound(baseline_successes, baseline_samples, cl)
        } else {
            baseline_rate
        };
        let derived = proportion::lower_bound_from_rate(effective_rate, test_samples, cl);

        if derived > target {
            // Threshold is still above target → need higher confidence
            // (which pushes the threshold down).
            lo = mid;
        } else {
            hi = mid;
        }
    }

    f64::midpoint(lo, hi)
}

#[cfg(test)]
#[allow(unused_must_use, reason = "test boilerplate may drop must_use values")]
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
    fn smaller_test_samples_produce_lower_threshold() {
        // Companion §3.5: the threshold is the Wilson lower bound at the
        // *test* sample size. A smaller test produces a wider interval
        // and therefore a lower bound, regardless of baseline size.
        let small_test = derive_sample_size_first(900, 1000, 50, cl(0.95));
        let large_test = derive_sample_size_first(900, 1000, 200, cl(0.95));
        assert!(large_test.value() > small_test.value());
    }

    #[test]
    fn perfect_baseline_does_not_force_threshold_to_one() {
        // Companion §4.3.2: when k = n in the baseline, the two-step
        // construction collapses the raw rate of 1.0 to a Wilson lower
        // bound at n_baseline before applying Wilson at n_test, keeping
        // the derived threshold strictly below 1.0.
        let dt = derive_sample_size_first(1000, 1000, 100, cl(0.95));
        assert!(dt.value() < 1.0);
        assert!(dt.value() > 0.9);
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
