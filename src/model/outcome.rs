//! Contract violations and the per-postcondition outcome alias.

use std::fmt;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_violation_displays_check_and_reason() {
        let v = ContractViolation::new("parse", "invalid JSON");
        assert_eq!(v.to_string(), "parse: invalid JSON");
    }
}
