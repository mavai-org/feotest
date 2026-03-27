//! Use case outcome: the result of executing a trial and evaluating its contract.

use std::time::{Duration, Instant};

use crate::contract::ServiceContract;
use crate::model::{ContractViolation, TrialOutcome};

/// The result of executing a use case trial and evaluating its contract.
///
/// Bundles the service response with contract evaluation results and timing.
/// This is the type returned from trial closures and consumed by the
/// execution engine.
///
/// # Examples
///
/// ```
/// use feotest::contract::{ServiceContract, UseCaseOutcome};
/// use feotest::model::ContractViolation;
///
/// let contract = ServiceContract::<String, u32>::builder()
///     .ensure("Is positive", |_input, response| {
///         if *response > 0 {
///             Ok(())
///         } else {
///             Err(ContractViolation::new("positive", "must be > 0"))
///         }
///     })
///     .build();
///
/// let outcome = UseCaseOutcome::evaluate(&contract, &"req".into(), || 42);
/// assert!(outcome.is_success());
/// assert_eq!(*outcome.response(), 42);
///
/// let outcome = UseCaseOutcome::evaluate(&contract, &"req".into(), || 0);
/// assert!(!outcome.is_success());
/// assert_eq!(outcome.violation().unwrap().check(), "positive");
/// ```
pub struct UseCaseOutcome<R> {
    response: R,
    trial_outcome: TrialOutcome,
}

impl<R> UseCaseOutcome<R> {
    /// Executes a service call and evaluates the contract against the result.
    ///
    /// Times the service call and evaluates all postconditions.
    pub fn evaluate<I>(
        contract: &ServiceContract<I, R>,
        input: &I,
        service_call: impl FnOnce() -> R,
    ) -> Self {
        let start = Instant::now();
        let response = service_call();
        let elapsed = start.elapsed();

        let outcome = contract.evaluate(input, &response);
        let trial_outcome = TrialOutcome::from_outcome(outcome, elapsed);

        Self {
            response,
            trial_outcome,
        }
    }

    /// Creates an outcome from a pre-computed response and explicit timing.
    ///
    /// Useful when the caller manages timing externally.
    pub fn from_response<I>(
        contract: &ServiceContract<I, R>,
        input: &I,
        response: R,
        elapsed: Duration,
    ) -> Self {
        let outcome = contract.evaluate(input, &response);
        let trial_outcome = TrialOutcome::from_outcome(outcome, elapsed);

        Self {
            response,
            trial_outcome,
        }
    }

    /// The trial outcome (success/failure with timing).
    #[must_use]
    pub const fn trial_outcome(&self) -> &TrialOutcome {
        &self.trial_outcome
    }

    /// The raw service response.
    #[must_use]
    pub const fn response(&self) -> &R {
        &self.response
    }

    /// Whether the contract was satisfied.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.trial_outcome.is_success()
    }

    /// The contract violation, if any.
    #[must_use]
    pub fn violation(&self) -> Option<&ContractViolation> {
        self.trial_outcome.violation()
    }

    /// Asserts that the contract was satisfied.
    ///
    /// # Panics
    ///
    /// Panics if any postcondition failed, with a message describing the violation.
    pub fn assert_contract(&self) {
        if let Some(violation) = self.violation() {
            panic!("Contract violation: {violation}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ServiceContract;
    use crate::model::ContractViolation;

    #[test]
    fn evaluate_successful_trial() {
        let contract = ServiceContract::<String, String>::builder()
            .ensure("has content", |_input, response| {
                if response.is_empty() {
                    Err(ContractViolation::new("content", "empty"))
                } else {
                    Ok(())
                }
            })
            .build();

        let outcome =
            UseCaseOutcome::evaluate(&contract, &"input".to_string(), || "hello".to_string());

        assert!(outcome.is_success());
        assert_eq!(outcome.response(), "hello");
    }

    #[test]
    fn evaluate_failing_trial() {
        let contract = ServiceContract::<String, String>::builder()
            .ensure("has content", |_input, response| {
                if response.is_empty() {
                    Err(ContractViolation::new("content", "empty"))
                } else {
                    Ok(())
                }
            })
            .build();

        let outcome = UseCaseOutcome::evaluate(&contract, &"input".to_string(), String::new);

        assert!(!outcome.is_success());
        assert_eq!(outcome.violation().unwrap().check(), "content");
    }

    #[test]
    #[should_panic(expected = "Contract violation: content: empty")]
    fn assert_contract_panics_on_violation() {
        let contract = ServiceContract::<String, String>::builder()
            .ensure("has content", |_input, _response| {
                Err(ContractViolation::new("content", "empty"))
            })
            .build();

        let outcome = UseCaseOutcome::evaluate(&contract, &"input".to_string(), String::new);
        outcome.assert_contract();
    }

    #[test]
    fn from_response_with_explicit_timing() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure("positive", |_input, response| {
                if *response > 0 {
                    Ok(())
                } else {
                    Err(ContractViolation::new("positive", "must be positive"))
                }
            })
            .build();

        let outcome = UseCaseOutcome::from_response(&contract, &1, 42, Duration::from_millis(100));
        assert!(outcome.is_success());
        assert_eq!(
            outcome.trial_outcome().elapsed(),
            Duration::from_millis(100)
        );
    }
}
