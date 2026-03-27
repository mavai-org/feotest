//! Pre-flight feasibility checks for sample sizing.
//!
//! Determines whether a configured sample size is large enough to produce
//! verification-grade evidence for a given target proportion.

use crate::statistics::proportion;
use crate::statistics::types::{ConfidenceLevel, FeasibilityResult};

/// The statistical criterion used for feasibility assessment.
const CRITERION: &str = "Wilson score one-sided lower bound";

/// Checks whether a sample size is too small for compliance-grade evidence.
///
/// A sample is undersized if, even with a perfect observation (all trials
/// succeed), the Wilson one-sided lower bound at significance level `alpha`
/// still falls below the `target`.
///
/// # Panics
///
/// Panics if `target` is not in [0, 1] or `alpha` is not in (0, 1).
#[must_use]
pub fn is_undersized(samples: u32, target: f64, alpha: f64) -> bool {
    assert!(
        (0.0..=1.0).contains(&target),
        "target must be in [0, 1], got {target}"
    );
    let confidence = ConfidenceLevel::new(1.0 - alpha);

    if samples == 0 {
        return true;
    }

    // Perfect observation: all succeed
    let lb = proportion::lower_bound(samples, samples, confidence);
    lb < target
}

/// Performs a full feasibility check for the configured sample size.
///
/// Returns a [`FeasibilityResult`] that records whether the sample size is
/// sufficient, the minimum required sample size, and the parameters used.
///
/// # Panics
///
/// Panics if `target` is not in [0, 1].
#[must_use]
pub fn feasibility_check(
    samples: u32,
    target: f64,
    confidence: ConfidenceLevel,
) -> FeasibilityResult {
    assert!(
        (0.0..=1.0).contains(&target),
        "target must be in [0, 1], got {target}"
    );

    let alpha = confidence.alpha();
    let feasible = !is_undersized(samples, target, alpha);
    let minimum = find_minimum_samples(target, confidence);

    FeasibilityResult::new(
        feasible,
        minimum,
        alpha,
        target,
        samples,
        CRITERION.to_owned(),
    )
}

/// Binary search for the minimum sample size at which a perfect observation
/// produces a Wilson lower bound ≥ target.
fn find_minimum_samples(target: f64, confidence: ConfidenceLevel) -> u32 {
    if target <= 0.0 {
        return 1;
    }

    // Upper bound: start searching up to a reasonable maximum.
    // For very high targets (e.g. 0.999) this may need to be large.
    let mut lo: u32 = 1;
    let mut hi: u32 = 10;

    // Expand hi until the lower bound at hi is sufficient.
    while hi < u32::MAX / 2 {
        let lb = proportion::lower_bound(hi, hi, confidence);
        if lb >= target {
            break;
        }
        lo = hi;
        hi = hi.saturating_mul(2);
    }

    // Binary search within [lo, hi]
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let lb = proportion::lower_bound(mid, mid, confidence);
        if lb >= target {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }

    lo
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use crate::statistics::defaults;

    fn cl(v: f64) -> ConfidenceLevel {
        ConfidenceLevel::new(v)
    }

    // --- is_undersized ---

    #[test]
    fn zero_samples_is_always_undersized() {
        assert!(is_undersized(0, 0.9, defaults::DEFAULT_ALPHA));
    }

    #[test]
    fn one_sample_is_undersized_for_high_target() {
        assert!(is_undersized(1, 0.9, defaults::DEFAULT_ALPHA));
    }

    #[test]
    fn large_sample_is_not_undersized() {
        assert!(!is_undersized(1000, 0.9, defaults::DEFAULT_ALPHA));
    }

    #[test]
    #[should_panic(expected = "target must be in")]
    fn undersized_panics_on_invalid_target() {
        is_undersized(100, 1.5, defaults::DEFAULT_ALPHA);
    }

    // --- feasibility_check ---

    #[test]
    fn feasible_with_sufficient_samples() {
        let result = feasibility_check(1000, 0.9, cl(0.95));
        assert!(result.feasible());
        assert!(result.minimum_samples() <= 1000);
    }

    #[test]
    fn not_feasible_with_tiny_sample() {
        let result = feasibility_check(5, 0.9, cl(0.95));
        assert!(!result.feasible());
        assert!(result.minimum_samples() > 5);
    }

    #[test]
    fn minimum_samples_is_sufficient() {
        let result = feasibility_check(100, 0.9, cl(0.95));
        let min = result.minimum_samples();
        let check = feasibility_check(min, 0.9, cl(0.95));
        assert!(check.feasible());
    }

    #[test]
    fn minimum_minus_one_is_not_sufficient() {
        let result = feasibility_check(100, 0.9, cl(0.95));
        let min = result.minimum_samples();
        if min > 1 {
            let check = feasibility_check(min - 1, 0.9, cl(0.95));
            assert!(!check.feasible());
        }
    }

    #[test]
    fn records_criterion() {
        let result = feasibility_check(100, 0.9, cl(0.95));
        assert_eq!(result.criterion(), "Wilson score one-sided lower bound");
    }

    #[test]
    fn records_configured_parameters() {
        let result = feasibility_check(100, 0.9, cl(0.95));
        assert_eq!(result.configured_samples(), 100);
        assert!((result.target() - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn higher_target_requires_more_samples() {
        let low = feasibility_check(1000, 0.8, cl(0.95));
        let high = feasibility_check(1000, 0.95, cl(0.95));
        assert!(high.minimum_samples() >= low.minimum_samples());
    }

    #[test]
    fn higher_confidence_requires_more_samples() {
        let low = feasibility_check(1000, 0.9, cl(0.90));
        let high = feasibility_check(1000, 0.9, cl(0.99));
        assert!(high.minimum_samples() >= low.minimum_samples());
    }
}
