//! Trial outcomes and contract violations.

use std::fmt;
use std::time::Duration;

/// A violation of a service contract postcondition.
///
/// Carries the name of the check that failed and a human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractViolation {
    check: String,
    reason: String,
}

impl ContractViolation {
    /// Creates a new contract violation.
    #[must_use]
    pub fn new(check: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            check: check.into(),
            reason: reason.into(),
        }
    }

    /// The name of the postcondition check that failed.
    #[must_use]
    pub fn check(&self) -> &str {
        &self.check
    }

    /// A human-readable explanation of why the check failed.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.check, self.reason)
    }
}

impl std::error::Error for ContractViolation {}

/// The result of evaluating a single postcondition check.
///
/// Defined as `Result<(), ContractViolation>` — a type alias grounding
/// the framework's contract evaluation in Rust's native error handling.
pub type Outcome = Result<(), ContractViolation>;

/// The result of a single trial execution, including timing and contract evaluation.
#[derive(Debug, Clone)]
pub struct TrialOutcome {
    outcome: Outcome,
    elapsed: Duration,
    metadata: Vec<(String, String)>,
}

impl TrialOutcome {
    /// Creates a successful trial outcome.
    #[must_use]
    pub const fn success(elapsed: Duration) -> Self {
        Self {
            outcome: Ok(()),
            elapsed,
            metadata: Vec::new(),
        }
    }

    /// Creates a failed trial outcome from a contract violation.
    #[must_use]
    pub const fn failure(violation: ContractViolation, elapsed: Duration) -> Self {
        Self {
            outcome: Err(violation),
            elapsed,
            metadata: Vec::new(),
        }
    }

    /// Creates a trial outcome from a `Result`.
    #[must_use]
    pub const fn from_outcome(outcome: Outcome, elapsed: Duration) -> Self {
        Self {
            outcome,
            elapsed,
            metadata: Vec::new(),
        }
    }

    /// Attaches a key-value metadata pair to this outcome.
    #[must_use]
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.push((key.into(), value.into()));
        self
    }

    /// Whether this trial succeeded.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.outcome.is_ok()
    }

    /// The contract violation, if any.
    #[must_use]
    pub fn violation(&self) -> Option<&ContractViolation> {
        self.outcome.as_ref().err()
    }

    /// How long the trial took to execute.
    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Metadata attached to this outcome.
    #[must_use]
    pub fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_trial_reports_success() {
        let trial = TrialOutcome::success(Duration::from_millis(42));
        assert!(trial.is_success());
        assert!(trial.violation().is_none());
        assert_eq!(trial.elapsed(), Duration::from_millis(42));
    }

    #[test]
    fn failed_trial_carries_violation() {
        let violation = ContractViolation::new("content", "empty response");
        let trial = TrialOutcome::failure(violation, Duration::from_millis(10));
        assert!(!trial.is_success());
        let v = trial.violation().unwrap();
        assert_eq!(v.check(), "content");
        assert_eq!(v.reason(), "empty response");
    }

    #[test]
    fn metadata_can_be_attached() {
        let trial = TrialOutcome::success(Duration::from_millis(1))
            .with_meta("model", "gpt-4o")
            .with_meta("tokens", "150");
        assert_eq!(trial.metadata().len(), 2);
        assert_eq!(
            trial.metadata()[0],
            ("model".to_string(), "gpt-4o".to_string())
        );
    }

    #[test]
    fn contract_violation_displays_check_and_reason() {
        let v = ContractViolation::new("parse", "invalid JSON");
        assert_eq!(v.to_string(), "parse: invalid JSON");
    }

    #[test]
    fn from_outcome_ok_is_success() {
        let trial = TrialOutcome::from_outcome(Ok(()), Duration::from_millis(5));
        assert!(trial.is_success());
    }

    #[test]
    fn from_outcome_err_is_failure() {
        let trial = TrialOutcome::from_outcome(
            Err(ContractViolation::new("check", "reason")),
            Duration::from_millis(5),
        );
        assert!(!trial.is_success());
    }
}
