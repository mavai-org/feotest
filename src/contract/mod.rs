//! Service contracts: postcondition criteria for individual invocations.
//!
//! A contract defines what it means for a single trial to meet or violate
//! its postconditions. A trial that violates a postcondition is a *contract
//! failure* — a legitimate statistical observation counted by the framework.
//! This is distinct from a software defect (a panic), which is not a contract
//! outcome and must not be conflated with one.
//!
//! The contract bridges between application-level postconditions and the
//! framework's statistical model by evaluating an ordered chain of checks
//! against a service response.

mod builder;
pub mod conformance;
mod duration;
mod evaluation;
#[cfg(feature = "json-matcher")]
pub mod json_matcher;

pub use builder::ServiceContractBuilder;
pub use conformance::{ConformanceResult, MatchResult, StringMatcher, VerificationMatcher};
pub use duration::{DurationConstraint, DurationResult};
pub use evaluation::UseCaseOutcome;

use crate::model::Outcome;

/// A service contract: an ordered chain of postcondition checks with an
/// optional duration constraint.
///
/// Postcondition checks are evaluated eagerly in declaration order (fail-fast).
/// Each `ensure` check returns `Result<(), ContractViolation>`.
///
/// Duration constraints are evaluated independently from postconditions,
/// providing a separate dimension of success/failure. Both "was it correct?"
/// and "was it fast enough?" are answered for every trial.
///
/// Contracts are constructed via [`ServiceContractBuilder`].
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use feotest::contract::ServiceContract;
/// use feotest::model::ContractViolation;
///
/// let contract = ServiceContract::<String, String>::builder()
///     .ensure("Not empty", |_input, response| {
///         if response.is_empty() {
///             Err(ContractViolation::new("content", "empty response"))
///         } else {
///             Ok(())
///         }
///     })
///     .ensure_duration_below(Duration::from_millis(500))
///     .build();
/// ```
pub struct ServiceContract<I, R> {
    checks: Vec<Check<I, R>>,
    duration_constraint: Option<DurationConstraint>,
}

impl<I, R> ServiceContract<I, R> {
    /// Starts building a new service contract.
    #[must_use]
    pub const fn builder() -> ServiceContractBuilder<I, R> {
        ServiceContractBuilder::new()
    }

    /// Evaluates all postcondition checks against the given input and response.
    ///
    /// Returns the first violation encountered, or `Ok(())` if all checks pass.
    /// This evaluates postcondition checks only; duration constraints are
    /// evaluated separately via [`DurationConstraint::evaluate`].
    ///
    /// # Errors
    ///
    /// Returns `Err(ContractViolation)` if any postcondition check fails.
    pub fn evaluate(&self, input: &I, response: &R) -> Outcome {
        for check in &self.checks {
            (check.f)(input, response)?;
        }
        Ok(())
    }

    /// The duration constraint, if any.
    ///
    /// Duration constraints are evaluated independently from postconditions,
    /// providing a parallel dimension of success/failure for timing requirements.
    #[must_use]
    pub const fn duration_constraint(&self) -> Option<&DurationConstraint> {
        self.duration_constraint.as_ref()
    }

    /// Returns the names of all postcondition checks in evaluation order.
    #[must_use]
    pub fn check_names(&self) -> Vec<&str> {
        self.checks.iter().map(|c| c.name.as_str()).collect()
    }
}

/// A named postcondition check within a service contract.
struct Check<I, R> {
    name: String,
    #[allow(
        clippy::type_complexity,
        reason = "trait-object signature captures the check contract"
    )]
    f: Box<dyn Fn(&I, &R) -> Outcome + Send + Sync>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ContractViolation;

    #[test]
    fn empty_contract_passes() {
        let contract = ServiceContract::<String, String>::builder().build();
        assert!(
            contract
                .evaluate(&"input".to_string(), &"response".to_string())
                .is_ok()
        );
    }

    #[test]
    fn single_passing_check() {
        let contract = ServiceContract::<String, String>::builder()
            .ensure("always passes", |_input, _response| Ok(()))
            .build();
        assert!(
            contract
                .evaluate(&"input".to_string(), &"response".to_string())
                .is_ok()
        );
    }

    #[test]
    fn single_failing_check() {
        let contract = ServiceContract::<String, String>::builder()
            .ensure("always fails", |_input, _response| {
                Err(ContractViolation::new("check", "forced failure"))
            })
            .build();
        let result = contract.evaluate(&"input".to_string(), &"response".to_string());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().check(), "check");
    }

    #[test]
    fn fail_fast_on_first_violation() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        let counter = Arc::new(AtomicU32::new(0));

        let c1 = Arc::clone(&counter);
        let c2 = Arc::clone(&counter);
        let contract = ServiceContract::<String, String>::builder()
            .ensure("first", move |_input, _response| {
                c1.fetch_add(1, Ordering::SeqCst);
                Err(ContractViolation::new("first", "fails"))
            })
            .ensure("second", move |_input, _response| {
                c2.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .build();

        let result = contract.evaluate(&"input".to_string(), &"response".to_string());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().check(), "first");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn checks_receive_input_and_response() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure("sum check", |input, response| {
                if input + response > 10 {
                    Ok(())
                } else {
                    Err(ContractViolation::new("sum", "too small"))
                }
            })
            .build();

        assert!(contract.evaluate(&5, &6).is_ok());
        assert!(contract.evaluate(&3, &2).is_err());
    }

    #[test]
    fn contract_without_duration_constraint_has_none() {
        let contract = ServiceContract::<u32, u32>::builder().build();
        assert!(contract.duration_constraint().is_none());
    }

    #[test]
    fn contract_with_duration_constraint() {
        use std::time::Duration;
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure_duration_below(Duration::from_millis(500))
            .build();
        let constraint = contract.duration_constraint().unwrap();
        assert_eq!(constraint.max_duration(), Duration::from_millis(500));
    }

    #[test]
    fn contract_with_custom_duration_description() {
        use std::time::Duration;
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure_duration_below_with_description("SLA limit", Duration::from_secs(1))
            .build();
        let constraint = contract.duration_constraint().unwrap();
        assert_eq!(constraint.description(), "SLA limit");
        assert_eq!(constraint.max_duration(), Duration::from_secs(1));
    }

    #[test]
    fn check_names_returns_names_in_order() {
        let contract = ServiceContract::<String, String>::builder()
            .ensure("alpha", |_input, _response| Ok(()))
            .ensure("beta", |_input, _response| Ok(()))
            .ensure("gamma", |_input, _response| Ok(()))
            .build();
        assert_eq!(contract.check_names(), vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn check_names_empty_contract() {
        let contract = ServiceContract::<String, String>::builder().build();
        assert!(contract.check_names().is_empty());
    }

    #[test]
    fn duration_constraint_chains_with_ensure() {
        use std::time::Duration;
        let contract = ServiceContract::<String, String>::builder()
            .ensure("has content", |_input, response| {
                if response.is_empty() {
                    Err(ContractViolation::new("content", "empty"))
                } else {
                    Ok(())
                }
            })
            .ensure_duration_below(Duration::from_millis(500))
            .build();

        // postconditions still work
        assert!(
            contract
                .evaluate(&"input".to_string(), &"hello".to_string())
                .is_ok()
        );
        assert!(
            contract
                .evaluate(&"input".to_string(), &String::new())
                .is_err()
        );
        // duration constraint is present
        assert!(contract.duration_constraint().is_some());
    }
}
