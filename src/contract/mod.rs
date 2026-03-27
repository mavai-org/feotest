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
mod evaluation;

pub use builder::ServiceContractBuilder;
pub use evaluation::UseCaseOutcome;

use crate::model::Outcome;

/// A service contract: an ordered chain of postcondition checks.
///
/// Checks are evaluated eagerly in declaration order (fail-fast).
/// Each `ensure` check returns `Result<(), ContractViolation>`.
///
/// Contracts are constructed via [`ServiceContractBuilder`].
///
/// # Examples
///
/// ```
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
///     .build();
/// ```
pub struct ServiceContract<I, R> {
    checks: Vec<Check<I, R>>,
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
}

/// A named postcondition check within a service contract.
struct Check<I, R> {
    #[allow(dead_code)]
    name: String,
    #[allow(clippy::type_complexity)]
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
}
