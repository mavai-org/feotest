//! Duration constraints for service contracts.
//!
//! A duration constraint defines the maximum acceptable execution time for a
//! service invocation. It is evaluated independently from postcondition checks,
//! providing a separate dimension of success/failure: "was it correct?" and
//! "was it fast enough?" are answered independently for every trial.

use std::fmt;
use std::time::Duration;

/// A constraint on execution duration.
///
/// Duration constraints are evaluated independently from postconditions,
/// providing a parallel dimension of success/failure. This allows both
/// correctness and latency to be assessed for every trial, regardless of
/// whether one or both fail.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use feotest::contract::DurationConstraint;
///
/// let constraint = DurationConstraint::below(Duration::from_millis(500));
/// assert_eq!(constraint.max_duration(), Duration::from_millis(500));
/// assert_eq!(constraint.description(), "Duration below 500ms");
/// ```
///
/// ```
/// use std::time::Duration;
/// use feotest::contract::DurationConstraint;
///
/// let constraint = DurationConstraint::below_with_description(
///     "API response time",
///     Duration::from_secs(2),
/// );
/// assert_eq!(constraint.description(), "API response time");
/// ```
///
/// # Panics
///
/// Construction panics if `max_duration` is zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurationConstraint {
    description: String,
    max_duration: Duration,
}

impl DurationConstraint {
    /// Creates a duration constraint with a default description.
    ///
    /// # Panics
    ///
    /// Panics if `max_duration` is zero.
    #[must_use]
    pub fn below(max_duration: Duration) -> Self {
        assert!(
            !max_duration.is_zero(),
            "max_duration must be positive, got zero"
        );
        Self {
            description: format!("Duration below {}", format_duration(max_duration)),
            max_duration,
        }
    }

    /// Creates a duration constraint with a custom description.
    ///
    /// # Panics
    ///
    /// Panics if `max_duration` is zero.
    #[must_use]
    pub fn below_with_description(description: impl Into<String>, max_duration: Duration) -> Self {
        assert!(
            !max_duration.is_zero(),
            "max_duration must be positive, got zero"
        );
        Self {
            description: description.into(),
            max_duration,
        }
    }

    /// The human-readable description of this constraint.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The maximum allowed execution duration.
    #[must_use]
    pub const fn max_duration(&self) -> Duration {
        self.max_duration
    }

    /// Evaluates this constraint against an actual execution duration.
    #[must_use]
    pub fn evaluate(&self, actual: Duration) -> DurationResult {
        let passed = actual <= self.max_duration;
        DurationResult {
            description: self.description.clone(),
            limit: self.max_duration,
            actual,
            passed,
        }
    }
}

/// The result of evaluating a duration constraint.
///
/// Captures the constraint parameters, the actual execution time, and whether
/// the constraint was satisfied. This allows diagnostic output to show the full
/// picture regardless of pass/fail status.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use feotest::contract::DurationConstraint;
///
/// let constraint = DurationConstraint::below(Duration::from_millis(500));
///
/// let result = constraint.evaluate(Duration::from_millis(230));
/// assert!(result.passed());
/// assert_eq!(result.message(), "Duration below 500ms: 230ms (limit: 500ms)");
///
/// let result = constraint.evaluate(Duration::from_millis(847));
/// assert!(!result.passed());
/// assert_eq!(result.message(), "Duration below 500ms: 847ms exceeded limit of 500ms");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurationResult {
    description: String,
    limit: Duration,
    actual: Duration,
    passed: bool,
}

impl DurationResult {
    /// Whether the constraint was satisfied.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }

    /// Whether the constraint was violated.
    #[must_use]
    pub const fn failed(&self) -> bool {
        !self.passed
    }

    /// The constraint description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The maximum allowed duration.
    #[must_use]
    pub const fn limit(&self) -> Duration {
        self.limit
    }

    /// The actual execution duration.
    #[must_use]
    pub const fn actual(&self) -> Duration {
        self.actual
    }

    /// A human-readable message describing the result.
    ///
    /// For passing results: `"Description: 230ms (limit: 500ms)"`
    /// For failing results: `"Description: 847ms exceeded limit of 500ms"`
    #[must_use]
    pub fn message(&self) -> String {
        let actual_fmt = format_duration(self.actual);
        let limit_fmt = format_duration(self.limit);
        if self.passed {
            format!("{}: {actual_fmt} (limit: {limit_fmt})", self.description)
        } else {
            format!(
                "{}: {actual_fmt} exceeded limit of {limit_fmt}",
                self.description
            )
        }
    }
}

impl fmt::Display for DurationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

/// Formats a duration for human-readable display.
#[allow(clippy::cast_precision_loss)]
fn format_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1000 {
        format!("{millis}ms")
    } else if millis < 60_000 {
        format!("{:.1}s", millis as f64 / 1000.0)
    } else {
        format!("{:.1}m", millis as f64 / 60_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_creates_constraint_with_default_description() {
        let constraint = DurationConstraint::below(Duration::from_millis(500));
        assert_eq!(constraint.description(), "Duration below 500ms");
        assert_eq!(constraint.max_duration(), Duration::from_millis(500));
    }

    #[test]
    fn below_with_description_uses_custom_description() {
        let constraint =
            DurationConstraint::below_with_description("SLA response time", Duration::from_secs(2));
        assert_eq!(constraint.description(), "SLA response time");
        assert_eq!(constraint.max_duration(), Duration::from_secs(2));
    }

    #[test]
    #[should_panic(expected = "max_duration must be positive")]
    fn below_rejects_zero_duration() {
        let _ = DurationConstraint::below(Duration::ZERO);
    }

    #[test]
    #[should_panic(expected = "max_duration must be positive")]
    fn below_with_description_rejects_zero_duration() {
        let _ = DurationConstraint::below_with_description("test", Duration::ZERO);
    }

    #[test]
    fn evaluate_passes_when_actual_equals_limit() {
        let constraint = DurationConstraint::below(Duration::from_millis(500));
        let result = constraint.evaluate(Duration::from_millis(500));
        assert!(result.passed());
        assert!(!result.failed());
    }

    #[test]
    fn evaluate_passes_when_actual_below_limit() {
        let constraint = DurationConstraint::below(Duration::from_millis(500));
        let result = constraint.evaluate(Duration::from_millis(200));
        assert!(result.passed());
    }

    #[test]
    fn evaluate_fails_when_actual_exceeds_limit() {
        let constraint = DurationConstraint::below(Duration::from_millis(500));
        let result = constraint.evaluate(Duration::from_millis(847));
        assert!(result.failed());
        assert!(!result.passed());
    }

    #[test]
    fn result_message_for_passing_check() {
        let constraint = DurationConstraint::below(Duration::from_millis(500));
        let result = constraint.evaluate(Duration::from_millis(230));
        assert_eq!(
            result.message(),
            "Duration below 500ms: 230ms (limit: 500ms)"
        );
    }

    #[test]
    fn result_message_for_failing_check() {
        let constraint = DurationConstraint::below(Duration::from_millis(500));
        let result = constraint.evaluate(Duration::from_millis(847));
        assert_eq!(
            result.message(),
            "Duration below 500ms: 847ms exceeded limit of 500ms"
        );
    }

    #[test]
    fn result_display_matches_message() {
        let constraint = DurationConstraint::below(Duration::from_millis(500));
        let result = constraint.evaluate(Duration::from_millis(230));
        assert_eq!(result.to_string(), result.message());
    }

    #[test]
    fn format_duration_millis() {
        assert_eq!(format_duration(Duration::from_millis(42)), "42ms");
        assert_eq!(format_duration(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(Duration::from_millis(1000)), "1.0s");
        assert_eq!(format_duration(Duration::from_millis(2500)), "2.5s");
        assert_eq!(format_duration(Duration::from_millis(59_999)), "60.0s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(Duration::from_millis(60_000)), "1.0m");
        assert_eq!(format_duration(Duration::from_millis(90_000)), "1.5m");
    }

    #[test]
    fn result_carries_limit_and_actual() {
        let constraint = DurationConstraint::below(Duration::from_millis(500));
        let result = constraint.evaluate(Duration::from_millis(300));
        assert_eq!(result.limit(), Duration::from_millis(500));
        assert_eq!(result.actual(), Duration::from_millis(300));
        assert_eq!(result.description(), "Duration below 500ms");
    }

    #[test]
    fn evaluate_zero_actual_duration_passes() {
        let constraint = DurationConstraint::below(Duration::from_millis(100));
        let result = constraint.evaluate(Duration::ZERO);
        assert!(result.passed());
    }
}
