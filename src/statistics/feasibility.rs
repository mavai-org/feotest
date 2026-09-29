//! Verification feasibility of a normative design (Statistical Companion
//! §5.7.1).
//!
//! Answers "can a compliance test of this size pass at all?" before any
//! sample is spent. Under `compliance/exact-binomial` a pass is possible only
//! if the all-success outcome clears the exact test, `p_req^n ≤ alpha`, so
//! the minimum is `⌈ln alpha / ln p_req⌉`. Feasible means a pass is possible,
//! not that the design is adequately powered (§5.5).
//!
//! Under verification intent an infeasible design is refused before it runs
//! (`COMPLIANCE_INFEASIBLE`); under smoke intent it runs and reports that a
//! pass is not possible at this size.

use crate::statistics::compliance::minimum_feasible_samples;
use crate::statistics::types::FeasibilityResult;

/// The name of the feasibility criterion — the method identifier published
/// by the reference oracle, asserted verbatim by the conformance suite.
const CRITERION: &str = "exact_binomial_pass_possible";

/// Checks whether a normative design of `samples` can pass.
///
/// # Panics
///
/// Panics if `samples` is zero, or if `target` or `alpha` is not in the open
/// interval `(0, 1)`.
#[must_use]
// mavai-ref: JVI-M5YQ6RB — do not remove (resolves in mavai-orchestrator)
pub fn feasibility_check(samples: u32, target: f64, alpha: f64) -> FeasibilityResult {
    assert!(samples > 0, "samples must be positive");
    let minimum = minimum_feasible_samples(target, alpha);
    FeasibilityResult::new(
        samples >= minimum,
        minimum,
        alpha,
        target,
        samples,
        CRITERION.to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_design_at_the_minimum_is_feasible() {
        let result = feasibility_check(59, 0.95, 0.05);
        assert!(result.feasible());
        assert_eq!(result.minimum_samples(), 59);
    }

    #[test]
    fn a_design_below_the_minimum_is_not() {
        let result = feasibility_check(58, 0.95, 0.05);
        assert!(!result.feasible());
        assert_eq!(result.minimum_samples(), 59);
    }

    #[test]
    fn records_the_criterion_and_the_configuration() {
        let result = feasibility_check(100, 0.9, 0.05);
        assert_eq!(result.criterion(), "exact_binomial_pass_possible");
        assert_eq!(result.configured_samples(), 100);
        assert!((result.target() - 0.9).abs() < f64::EPSILON);
        assert!((result.configured_alpha() - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn higher_targets_need_more_samples() {
        let low = feasibility_check(1000, 0.8, 0.05);
        let high = feasibility_check(1000, 0.95, 0.05);
        assert!(high.minimum_samples() > low.minimum_samples());
    }

    #[test]
    #[should_panic(expected = "samples must be positive")]
    fn rejects_a_zero_sample_design() {
        let _ = feasibility_check(0, 0.9, 0.05);
    }
}
