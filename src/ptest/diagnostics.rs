//! Diagnostic messages for configuration and feasibility issues.
//!
//! Produces structured, actionable messages when a probabilistic test
//! configuration is refused or undersized, and the pre-run latency planning
//! warnings. Messages include the subject, configured versus required sizes,
//! and remediation guidance.

use std::fmt::Write;

use crate::latency::Percentile;
use crate::ptest::preflight::RefusedPart;
use crate::statistics::latency::{NondegeneracyPlanning, PrecedencePlanning};
use crate::statistics::types::FeasibilityResult;

/// Formats a target proportion as a percentage, suppressing trailing zeros.
///
/// Whole-number targets display without decimals (e.g., 0.95 → "95%").
/// Fractional targets display without trailing zeros (e.g., 0.999 → "99.9%").
fn format_target_percent(target: f64) -> String {
    let pct = target * 100.0;
    if (pct - pct.round()).abs() < 1e-9 {
        format!("{:.0}%", pct.round())
    } else {
        // Format with enough precision, then strip trailing zeros
        let s = format!("{pct:.6}");
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        format!("{trimmed}%")
    }
}

/// The smoke caveat of a requirement this size cannot demonstrate
/// (§5.7.4): a PASS here is not evidence of compliance, and the feasibility
/// minimum is the size that could demonstrate it.
#[must_use]
pub fn smoke_undersized_message(subject: &str, result: &FeasibilityResult) -> String {
    format!(
        "{subject}: this size cannot demonstrate compliance with {} (N = {}, need at least {}). \
         A PASS at this size is not evidence of compliance; the smoke result reports only \
         whether there is evidence of non-compliance.",
        format_target_percent(result.target()),
        result.configured_samples(),
        result.minimum_samples()
    )
}

/// The smoke caveat of a requirement no outcome of this size can fail
/// (§5.7.4).
#[must_use]
pub fn smoke_fail_not_possible_message(subject: &str) -> String {
    format!(
        "{subject}: FAIL not possible at this size; this result carries no evidence about \
         the service."
    )
}

/// The smoke caveat of a requirement this size could demonstrate (§5.7.4).
#[must_use]
pub fn smoke_pass_possible_message(subject: &str) -> String {
    format!(
        "{subject}: PASS is possible at this size. Consider verification intent for \
         evidential strength, and check the power statement."
    )
}

/// Produces the diagnostic of a refused configuration: every invalid part,
/// in the fixed code order, with the remediation for each code.
#[must_use]
pub fn refusal_message(test_name: &str, planned: u32, refused: &[RefusedPart]) -> String {
    let mut msg = format!(
        "Configuration refused for \"{test_name}\" before any sample ran \
         ({planned} samples planned):"
    );
    for part in refused {
        let _ = write!(msg, "\n  {}  {}", part.code(), part.describe());
    }
    msg.push_str(
        "\n  Remediation: a baseline must be at least as large as any test that consumes it \
         — plan a smaller test or measure a larger baseline; a requirement no outcome can \
         demonstrate needs more samples, or Smoke intent to ask only whether there is \
         evidence that the service falls short of it.",
    );
    msg
}

/// The planning warning of a baseline-derived latency constraint for which
/// no baseline rank is expected to achieve alpha (§12.5.3).
#[must_use]
pub fn saturation_message(
    percentile: Percentile,
    planning: &PrecedencePlanning,
    baseline_latencies: u32,
) -> String {
    let expected = planning.expected_test_samples();
    planning.minimum_baseline_trials().map_or_else(
        || {
            format!(
                "{percentile}: no successful latency is expected, so no threshold can be derived"
            )
        },
        |minimum| {
            format!(
                "{percentile}: with about {expected} successful latencies expected, no rank of the \
                 baseline's {baseline_latencies} latencies achieves alpha — the constraint is \
                 expected to be saturated (inconclusive); a baseline of at least {minimum} \
                 successful latencies supports it"
            )
        },
    )
}

/// The planning warning of a latency percentile expected to be degenerate
/// (§12.5.2, §12.5.3).
#[must_use]
pub fn degeneracy_message(percentile: Percentile, planning: &NondegeneracyPlanning) -> String {
    format!(
        "{percentile}: about {} successful latencies are expected, below the minimum of {} for a \
         non-degenerate {percentile}; plan at least {} samples",
        planning.expected_test_samples(),
        planning.minimum_contributing_samples(),
        planning.planned_samples_needed()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statistics::feasibility;
    use crate::statistics::latency::{plan_nondegeneracy, plan_precedence};

    fn feasibility_check(samples: u32, target: f64, confidence: f64) -> FeasibilityResult {
        feasibility::feasibility_check(samples, target, 1.0 - confidence)
    }

    fn cl(v: f64) -> f64 {
        v
    }

    #[test]
    fn whole_number_target_no_decimals() {
        let result = feasibility_check(5, 0.90, cl(0.95));
        let msg = smoke_undersized_message("test", &result);
        assert!(msg.contains("90%"), "expected '90%' in: {msg}");
        assert!(
            !msg.contains("90.0%"),
            "should not contain '90.0%' in: {msg}"
        );
    }

    #[test]
    fn fractional_target_no_trailing_zeros() {
        let result = feasibility_check(5, 0.999, cl(0.95));
        let msg = smoke_undersized_message("test", &result);
        assert!(msg.contains("99.9%"), "expected '99.9%' in: {msg}");
        assert!(
            !msg.contains("99.900%"),
            "should not contain '99.900%' in: {msg}"
        );
    }

    #[test]
    fn the_smoke_undersized_caveat_disclaims_compliance_and_names_the_minimum() {
        let result = feasibility_check(100, 0.99, cl(0.95));
        let msg = smoke_undersized_message("accuracy", &result);
        assert!(msg.contains("accuracy: this size cannot demonstrate compliance with 99%"));
        assert!(msg.contains("N = 100, need at least 299"));
        assert!(msg.contains("not evidence of compliance"));
    }

    #[test]
    fn the_smoke_mirror_caveats_name_their_subject() {
        assert_eq!(
            smoke_fail_not_possible_message("coin"),
            "coin: FAIL not possible at this size; this result carries no evidence about \
             the service."
        );
        assert!(smoke_pass_possible_message("coin").starts_with("coin: PASS is possible"));
    }

    #[test]
    fn saturation_warning_names_the_baseline_that_supports_the_constraint() {
        let planning = plan_precedence(100, 20, 0.75, 0.95, 0.05);
        let msg = saturation_message(Percentile::P95, &planning, 100);
        assert!(msg.contains("saturated"));
        assert!(msg.contains(&planning.minimum_baseline_trials().unwrap().to_string()));
    }

    #[test]
    fn degeneracy_warning_names_the_planned_size_needed() {
        let planning = plan_nondegeneracy(0.99, 110, 0.8);
        let msg = degeneracy_message(Percentile::P99, &planning);
        assert!(msg.contains("plan at least 125 samples"));
    }
}
