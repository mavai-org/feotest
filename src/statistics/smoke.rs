//! Normative compliance under smoke intent: `compliance/smoke-exact-binomial`
//! (Statistical Companion §5.7.3) and its latency form
//! `latency/smoke-compliance-exact-binomial` (§12.3.4).
//!
//! A smoke test asks the sentinel's question — *is there evidence that the
//! service falls short of its requirement?* — so the burden of proof is
//! reversed, at every sample size:
//!
//! ```text
//! H0: p ≥ p_req,  H1: p < p_req,  FAIL iff P_{p_req}(K ≤ k) ≤ alpha.
//! ```
//!
//! The lower tail rises with `k`, so the failing counts are `0, …, f` and the
//! binding count is `required_pass = f + 1`, or 0 when no count can fail;
//! the criterion passes iff `K ≥ required_pass`, as under every other rule.
//! A smoke pass is not evidence of compliance at any size: it states that
//! the run found no evidence of non-compliance. A smoke fail is evidence of
//! non-compliance at level alpha. The exact-boundary convention of §10.6
//! applies.

use crate::statistics::distributions::binomial_cdf;
use crate::statistics::exact::{at_most_alpha, binomial_lower_tail_exact};

/// Validates a requirement and a level.
fn validate(requirement: f64, alpha: f64) {
    assert!(
        requirement > 0.0 && requirement < 1.0,
        "the requirement must be strictly between 0 and 1, got {requirement}"
    );
    assert!(
        alpha > 0.0 && alpha < 1.0,
        "alpha must be strictly between 0 and 1, got {alpha}"
    );
}

/// `P_{p_req}(K ≤ count)`, `K ~ Bin(samples, p_req)`: the smoke rule's
/// p-value for an observed count.
///
/// # Panics
///
/// Panics if the requirement is outside `[0, 1]`.
#[must_use]
pub fn smoke_p_value(count: u32, samples: u32, requirement: f64) -> f64 {
    binomial_cdf(i64::from(count), samples, requirement)
}

/// Whether a count is evidence of non-compliance at this size: its lower
/// tail under the requirement is at most alpha (exact-boundary convention).
fn fails(count: u32, samples: u32, requirement: f64, alpha: f64) -> bool {
    at_most_alpha(smoke_p_value(count, samples, requirement), alpha, || {
        binomial_lower_tail_exact(count, samples, requirement)
    })
}

/// `required_pass` of `compliance/smoke-exact-binomial`: one more than the
/// largest failing count, 0 when no count can fail at this size.
///
/// # Panics
///
/// Panics if `samples` is zero, or the requirement or level is outside
/// `(0, 1)`.
#[must_use]
pub fn smoke_required_pass(requirement: f64, samples: u32, alpha: f64) -> u32 {
    assert!(samples > 0, "samples must be positive");
    validate(requirement, alpha);
    if !fails(0, samples, requirement, alpha) {
        return 0;
    }
    // The lower tail rises with the count: bisect for the first count that
    // no longer fails. `samples` itself never fails (its lower tail is 1).
    let (mut low, mut high) = (1_u32, samples);
    while low < high {
        let mid = low + (high - low) / 2;
        if fails(mid, samples, requirement, alpha) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

/// `P_{p_req}(K < required_pass)`: the probability of a FAIL when the
/// service exactly meets its requirement, at most alpha; 0 when no count
/// can fail.
///
/// # Panics
///
/// Panics if the requirement is outside `[0, 1]`.
#[must_use]
pub fn false_non_compliance(required_pass: u32, samples: u32, requirement: f64) -> f64 {
    if required_pass == 0 {
        return 0.0;
    }
    smoke_p_value(required_pass - 1, samples, requirement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn required_pass_matches_the_companion_worked_values() {
        let cases = [
            (0.99, 100, 0.05, 97),
            (0.99, 30, 0.05, 29),
            (0.99, 20, 0.05, 19),
            (0.99, 5, 0.05, 5),
            (0.9, 5, 0.05, 3),
            (0.5, 1, 0.05, 0),
            (0.995, 477, 0.05, 472),
            (0.95, 150, 0.05, 138),
            (0.95, 50, 0.05, 45),
            (0.95, 58, 0.05, 52),
        ];
        for (requirement, samples, alpha, expected) in cases {
            assert_eq!(
                smoke_required_pass(requirement, samples, alpha),
                expected,
                "p_req {requirement}, n {samples}"
            );
        }
    }

    #[test]
    fn the_exact_boundary_fails_inclusively() {
        // P(K <= 0) = 1/32 = alpha exactly: 0 of 5 fails, so 1 is required.
        assert_eq!(smoke_required_pass(0.5, 5, 0.031_25), 1);
    }

    #[test]
    fn the_largest_failing_count_is_at_most_alpha_and_the_next_is_above() {
        let required = smoke_required_pass(0.99, 100, 0.05);
        assert!(smoke_p_value(required - 1, 100, 0.99) <= 0.05);
        assert!(smoke_p_value(required, 100, 0.99) > 0.05);
    }

    #[test]
    fn false_non_compliance_is_the_tail_below_the_required_count() {
        assert_relative_eq!(
            false_non_compliance(97, 100, 0.99),
            0.018_374_036_444_649_7,
            max_relative = 1e-10
        );
        assert_relative_eq!(false_non_compliance(0, 1, 0.5), 0.0);
    }

    #[test]
    #[should_panic(expected = "samples must be positive")]
    fn rejects_an_empty_design() {
        let _ = smoke_required_pass(0.9, 0, 0.05);
    }
}
