//! Diagnostic messages for configuration and feasibility issues.
//!
//! Produces structured, actionable messages when a probabilistic test
//! configuration is infeasible. Messages include the test identity,
//! configured vs required sample sizes, and remediation guidance.

use std::fmt::Write;

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
/// Appends the Wilson score criterion, alpha, and confidence level.
#[must_use]
pub fn infeasibility_message(
    test_name: &str,
    result: &FeasibilityResult,
    verbose: bool,
) -> String {
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
             \x20 Assessment:  even with {configured}/{configured} successes, \
             Wilson lower bound < {target:.3}",
            criterion = result.criterion(),
            target = result.target(),
        );
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statistics::feasibility::feasibility_check;
    use crate::statistics::types::ConfidenceLevel;

    fn cl(v: f64) -> ConfidenceLevel {
        ConfidenceLevel::new(v)
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
        assert!(!msg.contains("90.0%"), "should not contain '90.0%' in: {msg}");
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
        assert!(msg.contains("Wilson score"));
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
        assert!(!msg.contains("Criterion"), "default mode should not include criterion");
        assert!(!msg.contains("Alpha"), "default mode should not include alpha");
    }
}
