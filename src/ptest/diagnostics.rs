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

/// Produces a structured infeasibility diagnostic.
///
/// # Default mode (`verbose: false`)
///
/// Reports the test name, configured samples, target, minimum required N,
/// and remediation guidance.
///
/// # Verbose mode (`verbose: true`)
///
/// Appends the feasibility criterion, alpha, and confidence level.
#[must_use]
pub fn infeasibility_message(test_name: &str, result: &FeasibilityResult, verbose: bool) -> String {
    let target_pct = format_target_percent(result.target());
    let configured = result.configured_samples();
    let minimum = result.minimum_samples();

    let mut msg = format!(
        "Infeasible configuration for \"{test_name}\":\n\
         \x20 Configured:  {configured} samples at target {target_pct}\n\
         \x20 Minimum:     {minimum} samples required for verification-grade evidence\n\
         \x20 Remediation: increase samples to at least {minimum},\n\
         \x20              or use Smoke intent to proceed with reduced confidence."
    );

    if verbose {
        let alpha = result.configured_alpha();
        let confidence_pct = format_target_percent(1.0 - alpha);
        let _ = write!(
            msg,
            "\n\
             \x20 Criterion:   {criterion}\n\
             \x20 Alpha:       {alpha:.3}\n\
             \x20 Confidence:  {confidence_pct}\n\
             \x20 Assessment:  even {configured}/{configured} successes cannot demonstrate \
             {target:.3}: {target:.3}^{configured} > alpha",
            criterion = result.criterion(),
            target = result.target(),
        );
    }

    msg
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
         demonstrate needs more samples, or Smoke intent to run it without evidential weight.",
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
    fn includes_test_name() {
        let result = feasibility_check(5, 0.95, cl(0.95));
        let msg = infeasibility_message("shopping-basket", &result, false);
        assert!(msg.contains("shopping-basket"));
    }

    #[test]
    fn includes_minimum_samples() {
        let result = feasibility_check(5, 0.95, cl(0.95));
        let msg = infeasibility_message("test", &result, false);
        let min = result.minimum_samples().to_string();
        assert!(msg.contains(&min));
    }

    #[test]
    fn includes_remediation() {
        let result = feasibility_check(5, 0.95, cl(0.95));
        let msg = infeasibility_message("test", &result, false);
        assert!(msg.contains("increase samples"));
        assert!(msg.contains("Smoke intent"));
    }

    #[test]
    fn whole_number_target_no_decimals() {
        let result = feasibility_check(5, 0.90, cl(0.95));
        let msg = infeasibility_message("test", &result, false);
        assert!(msg.contains("90%"), "expected '90%' in: {msg}");
        assert!(
            !msg.contains("90.0%"),
            "should not contain '90.0%' in: {msg}"
        );
    }

    #[test]
    fn fractional_target_no_trailing_zeros() {
        let result = feasibility_check(5, 0.999, cl(0.95));
        let msg = infeasibility_message("test", &result, false);
        assert!(msg.contains("99.9%"), "expected '99.9%' in: {msg}");
        assert!(
            !msg.contains("99.900%"),
            "should not contain '99.900%' in: {msg}"
        );
    }

    #[test]
    fn verbose_includes_criterion() {
        let result = feasibility_check(5, 0.95, cl(0.95));
        let msg = infeasibility_message("test", &result, true);
        assert!(msg.contains("exact_binomial_pass_possible"));
    }

    #[test]
    fn verbose_includes_alpha() {
        let result = feasibility_check(5, 0.95, cl(0.95));
        let msg = infeasibility_message("test", &result, true);
        assert!(msg.contains("0.050"));
    }

    #[test]
    fn default_mode_excludes_verbose_details() {
        let result = feasibility_check(5, 0.95, cl(0.95));
        let msg = infeasibility_message("test", &result, false);
        assert!(
            !msg.contains("Criterion"),
            "default mode should not include criterion"
        );
        assert!(
            !msg.contains("Alpha"),
            "default mode should not include alpha"
        );
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
