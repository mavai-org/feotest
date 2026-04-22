//! Test verdict evaluation.
//!
//! Evaluates observed test outcomes against a derived threshold and produces
//! a [`VerdictWithConfidence`] that includes the pass/fail decision, the
//! observed rate, and the false positive probability.

use crate::statistics::types::{DerivedThreshold, VerdictWithConfidence};

/// Evaluates test results against a derived threshold.
///
/// The test passes if the observed success rate (successes / samples) is
/// at or above the threshold value. The false positive probability is
/// the significance level α = 1 − confidence from the threshold's
/// derivation context.
///
/// # Panics
///
/// Panics if `test_samples` is zero or `test_successes > test_samples`.
#[must_use]
pub fn evaluate(
    test_successes: u32,
    test_samples: u32,
    threshold: &DerivedThreshold,
) -> VerdictWithConfidence {
    assert!(test_samples > 0, "test_samples must be positive");
    assert!(
        test_successes <= test_samples,
        "test_successes ({test_successes}) cannot exceed test_samples ({test_samples})"
    );

    let observed_rate = f64::from(test_successes) / f64::from(test_samples);
    let passed = observed_rate >= threshold.value();
    let false_positive_probability = threshold.context().confidence().alpha();

    VerdictWithConfidence::new(
        passed,
        observed_rate,
        threshold.clone(),
        false_positive_probability,
    )
}

/// Summarises multiple independent verdict runs.
///
/// For independent failures, the combined false positive probability is
/// the product of individual false positive probabilities. A single failure
/// may be a false positive; repeated failures provide strong evidence of
/// genuine degradation.
///
/// Returns `None` if the slice is empty.
#[must_use]
pub fn summarize_multiple_runs(verdicts: &[VerdictWithConfidence]) -> Option<MultiRunSummary> {
    if verdicts.is_empty() {
        return None;
    }

    let total = verdicts.len();
    let passed = verdicts.iter().filter(|v| v.passed()).count();
    let failed = total - passed;

    let combined_false_positive_probability = if failed == 0 {
        0.0
    } else {
        verdicts
            .iter()
            .filter(|v| !v.passed())
            .map(super::types::VerdictWithConfidence::false_positive_probability)
            .product()
    };

    Some(MultiRunSummary {
        total_runs: total,
        passed_runs: passed,
        failed_runs: failed,
        combined_false_positive_probability,
    })
}

/// Summary of multiple independent test runs.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiRunSummary {
    /// Total number of runs evaluated.
    total_runs: usize,
    /// Number of runs that passed.
    passed_runs: usize,
    /// Number of runs that failed.
    failed_runs: usize,
    /// Combined probability that all failures are false positives.
    combined_false_positive_probability: f64,
}

impl MultiRunSummary {
    /// Total number of runs evaluated.
    #[must_use]
    pub const fn total_runs(&self) -> usize {
        self.total_runs
    }

    /// Number of runs that passed.
    #[must_use]
    pub const fn passed_runs(&self) -> usize {
        self.passed_runs
    }

    /// Number of runs that failed.
    #[must_use]
    pub const fn failed_runs(&self) -> usize {
        self.failed_runs
    }

    /// Combined probability that all failures are false positives.
    ///
    /// For independent runs, this is the product of individual false
    /// positive probabilities for the failing runs.
    #[must_use]
    pub const fn combined_false_positive_probability(&self) -> f64 {
        self.combined_false_positive_probability
    }
}

#[cfg(test)]
#[allow(unused_must_use, reason = "test boilerplate may drop must_use values")]
mod tests {
    use super::*;
    use crate::statistics::types::{ConfidenceLevel, DerivationContext, OperationalApproach};
    use approx::assert_relative_eq;

    fn make_threshold(threshold_value: f64, confidence: f64) -> DerivedThreshold {
        let cl = ConfidenceLevel::new(confidence);
        let ctx = DerivationContext::new(0.9, 100, 100, cl);
        DerivedThreshold::new(
            threshold_value,
            OperationalApproach::SampleSizeFirst,
            ctx,
            true,
        )
    }

    // --- evaluate ---

    #[test]
    fn passes_when_observed_rate_above_threshold() {
        let threshold = make_threshold(0.80, 0.95);
        let verdict = evaluate(90, 100, &threshold);
        assert!(verdict.passed());
        assert_relative_eq!(verdict.observed_rate(), 0.9, epsilon = 1e-10);
    }

    #[test]
    fn passes_when_observed_rate_equals_threshold() {
        let threshold = make_threshold(0.80, 0.95);
        let verdict = evaluate(80, 100, &threshold);
        assert!(verdict.passed());
    }

    #[test]
    fn fails_when_observed_rate_below_threshold() {
        let threshold = make_threshold(0.80, 0.95);
        let verdict = evaluate(70, 100, &threshold);
        assert!(!verdict.passed());
        assert_relative_eq!(verdict.observed_rate(), 0.7, epsilon = 1e-10);
    }

    #[test]
    fn shortfall_is_zero_on_pass() {
        let threshold = make_threshold(0.80, 0.95);
        let verdict = evaluate(90, 100, &threshold);
        assert_relative_eq!(verdict.shortfall(), 0.0, epsilon = 1e-10);
    }

    #[test]
    fn shortfall_is_positive_on_failure() {
        let threshold = make_threshold(0.80, 0.95);
        let verdict = evaluate(70, 100, &threshold);
        assert_relative_eq!(verdict.shortfall(), 0.10, epsilon = 1e-10);
    }

    #[test]
    fn false_positive_probability_matches_alpha() {
        let threshold = make_threshold(0.80, 0.95);
        let verdict = evaluate(90, 100, &threshold);
        assert_relative_eq!(verdict.false_positive_probability(), 0.05, epsilon = 1e-10);
    }

    #[test]
    fn confidence_is_complement_of_false_positive() {
        let threshold = make_threshold(0.80, 0.95);
        let verdict = evaluate(90, 100, &threshold);
        assert_relative_eq!(verdict.confidence(), 0.95, epsilon = 1e-10);
    }

    #[test]
    #[should_panic(expected = "test_samples must be positive")]
    fn panics_on_zero_test_samples() {
        let threshold = make_threshold(0.80, 0.95);
        evaluate(0, 0, &threshold);
    }

    #[test]
    #[should_panic(expected = "cannot exceed test_samples")]
    fn panics_on_successes_exceeding_samples() {
        let threshold = make_threshold(0.80, 0.95);
        evaluate(101, 100, &threshold);
    }

    // --- summarize_multiple_runs ---

    #[test]
    fn summary_of_empty_runs_is_none() {
        assert!(summarize_multiple_runs(&[]).is_none());
    }

    #[test]
    fn summary_all_pass() {
        let threshold = make_threshold(0.80, 0.95);
        let v1 = evaluate(90, 100, &threshold);
        let v2 = evaluate(85, 100, &threshold);
        let summary = summarize_multiple_runs(&[v1, v2]).unwrap();
        assert_eq!(summary.total_runs(), 2);
        assert_eq!(summary.passed_runs(), 2);
        assert_eq!(summary.failed_runs(), 0);
        assert_relative_eq!(
            summary.combined_false_positive_probability(),
            0.0,
            epsilon = 1e-10
        );
    }

    #[test]
    fn summary_single_failure() {
        let threshold = make_threshold(0.80, 0.95);
        let pass = evaluate(90, 100, &threshold);
        let fail = evaluate(70, 100, &threshold);
        let summary = summarize_multiple_runs(&[pass, fail]).unwrap();
        assert_eq!(summary.failed_runs(), 1);
        assert_relative_eq!(
            summary.combined_false_positive_probability(),
            0.05,
            epsilon = 1e-10
        );
    }

    #[test]
    fn summary_repeated_failures_multiply_probabilities() {
        let threshold = make_threshold(0.80, 0.95);
        let f1 = evaluate(70, 100, &threshold);
        let f2 = evaluate(75, 100, &threshold);
        let summary = summarize_multiple_runs(&[f1, f2]).unwrap();
        assert_eq!(summary.failed_runs(), 2);
        // 0.05 × 0.05 = 0.0025
        assert_relative_eq!(
            summary.combined_false_positive_probability(),
            0.0025,
            epsilon = 1e-10
        );
    }
}
