//! Service contract outcome: the result of executing a trial and evaluating its contract.

use std::fmt;
use std::time::{Duration, Instant};

use crate::contract::ServiceContract;
use crate::contract::conformance::{ConformanceResult, VerificationMatcher};
use crate::contract::duration::DurationResult;
use crate::model::{ContractViolation, TrialOutcome};

/// The result of executing a service contract trial and evaluating its contract.
///
/// Bundles the service response with contract evaluation results, timing,
/// and duration constraint results. Postconditions and duration constraints
/// are evaluated independently — a trial can fail on either or both dimensions.
///
/// # Examples
///
/// ```
/// use feotest::contract::{ServiceContract, ServiceContractOutcome};
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
/// let outcome = ServiceContractOutcome::evaluate(&contract, &"req".into(), || 42);
/// assert!(outcome.is_success());
/// assert_eq!(*outcome.response(), 42);
///
/// let outcome = ServiceContractOutcome::evaluate(&contract, &"req".into(), || 0);
/// assert!(!outcome.is_success());
/// assert_eq!(outcome.violation().unwrap().check(), "positive");
/// ```
pub struct ServiceContractOutcome<R> {
    response: R,
    trial_outcome: TrialOutcome,
    duration_result: Option<DurationResult>,
    conformance_result: Option<ConformanceResult>,
}

impl<R> ServiceContractOutcome<R> {
    /// Executes a service call and evaluates the contract against the result.
    ///
    /// Times the service call and evaluates all postconditions and the
    /// duration constraint (if any).
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
        let duration_result = contract.duration_constraint().map(|c| c.evaluate(elapsed));

        Self {
            response,
            trial_outcome,
            duration_result,
            conformance_result: None,
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
        let duration_result = contract.duration_constraint().map(|c| c.evaluate(elapsed));

        Self {
            response,
            trial_outcome,
            duration_result,
            conformance_result: None,
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

    /// Whether the trial fully succeeded: all postconditions passed, the
    /// duration constraint (if any) was satisfied, and the conformance check
    /// (if any) matched.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.trial_outcome.is_success() && self.within_duration_limit() && self.matches_expected()
    }

    /// The postcondition violation, if any.
    #[must_use]
    pub fn violation(&self) -> Option<&ContractViolation> {
        self.trial_outcome.violation()
    }

    /// The duration constraint result, if a constraint was configured.
    #[must_use]
    pub const fn duration_result(&self) -> Option<&DurationResult> {
        self.duration_result.as_ref()
    }

    /// Whether the execution was within the duration limit.
    ///
    /// Returns `true` if no duration constraint is configured or if the
    /// actual duration was within the limit.
    #[must_use]
    pub fn within_duration_limit(&self) -> bool {
        self.duration_result
            .as_ref()
            .is_none_or(DurationResult::passed)
    }

    /// Attaches an instance conformance check to this outcome.
    ///
    /// The extractor projects the response into the matchable type, the matcher
    /// compares it against the expected value, and the result is stored for
    /// diagnostic reporting.
    #[must_use]
    pub fn conforms_to<T: fmt::Debug>(
        mut self,
        expected: &T,
        extractor: impl Fn(&R) -> T,
        matcher: &dyn VerificationMatcher<T>,
    ) -> Self {
        let actual = extractor(&self.response);
        let match_result = matcher.verify(expected, &actual);
        self.conformance_result = Some(ConformanceResult::new(
            format!("{expected:?}"),
            match_result,
        ));
        self
    }

    /// The conformance result, if a conformance check was specified.
    #[must_use]
    pub const fn conformance_result(&self) -> Option<&ConformanceResult> {
        self.conformance_result.as_ref()
    }

    /// Whether the actual value matched the expected value.
    ///
    /// Returns `true` if no conformance check was specified, or if it passed.
    #[must_use]
    pub fn matches_expected(&self) -> bool {
        self.conformance_result
            .as_ref()
            .is_none_or(ConformanceResult::is_match)
    }

    /// Asserts that all postconditions, the duration constraint, and the
    /// conformance check were satisfied.
    ///
    /// # Panics
    ///
    /// Panics if any postcondition failed, the conformance check mismatched,
    /// or the duration constraint was violated, with a message describing the
    /// failure.
    pub fn assert_contract(&self) {
        let mut failures: Vec<String> = Vec::new();

        if let Some(violation) = self.violation() {
            failures.push(format!(
                "Postcondition '{}' failed: {}",
                violation.check(),
                violation.reason()
            ));
        }

        if let Some(cr) = &self.conformance_result {
            if cr.match_result().is_mismatch() {
                failures.push(format!(
                    "Expected value mismatch: {}",
                    cr.match_result().diff()
                ));
            }
        }

        if let Some(dr) = &self.duration_result {
            if dr.failed() {
                failures.push(dr.message());
            }
        }

        assert!(
            failures.is_empty(),
            "Contract violations:\n  - {}",
            failures.join("\n  - ")
        );
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
            ServiceContractOutcome::evaluate(&contract, &"input".to_string(), || "hello".to_string());

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

        let outcome = ServiceContractOutcome::evaluate(&contract, &"input".to_string(), String::new);

        assert!(!outcome.is_success());
        assert_eq!(outcome.violation().unwrap().check(), "content");
    }

    #[test]
    #[should_panic(expected = "Postcondition 'content' failed: empty")]
    fn assert_contract_panics_on_postcondition_violation() {
        let contract = ServiceContract::<String, String>::builder()
            .ensure("has content", |_input, _response| {
                Err(ContractViolation::new("content", "empty"))
            })
            .build();

        let outcome = ServiceContractOutcome::evaluate(&contract, &"input".to_string(), String::new);
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

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(100));
        assert!(outcome.is_success());
        assert_eq!(
            outcome.trial_outcome().elapsed(),
            Duration::from_millis(100)
        );
    }

    // --- Duration constraint tests ---

    #[test]
    fn no_duration_constraint_means_no_duration_result() {
        let contract = ServiceContract::<u32, u32>::builder().build();
        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(100));
        assert!(outcome.duration_result().is_none());
        assert!(outcome.within_duration_limit());
    }

    #[test]
    fn duration_constraint_passes_when_within_limit() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure_duration_below(Duration::from_millis(500))
            .build();

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(200));

        assert!(outcome.within_duration_limit());
        assert!(outcome.is_success());
        let dr = outcome.duration_result().unwrap();
        assert!(dr.passed());
        assert_eq!(dr.actual(), Duration::from_millis(200));
        assert_eq!(dr.limit(), Duration::from_millis(500));
    }

    #[test]
    fn duration_constraint_fails_when_exceeding_limit() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure_duration_below(Duration::from_millis(500))
            .build();

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(800));

        assert!(!outcome.within_duration_limit());
        assert!(!outcome.is_success());
        let dr = outcome.duration_result().unwrap();
        assert!(dr.failed());
    }

    #[test]
    fn postcondition_pass_and_duration_fail_means_overall_failure() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure("always passes", |_input, _response| Ok(()))
            .ensure_duration_below(Duration::from_millis(100))
            .build();

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(200));

        assert!(outcome.violation().is_none()); // postconditions passed
        assert!(!outcome.within_duration_limit()); // duration failed
        assert!(!outcome.is_success()); // overall failure
    }

    #[test]
    fn postcondition_fail_and_duration_pass_means_overall_failure() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure("always fails", |_input, _response| {
                Err(ContractViolation::new("check", "forced"))
            })
            .ensure_duration_below(Duration::from_millis(500))
            .build();

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(100));

        assert!(outcome.violation().is_some()); // postconditions failed
        assert!(outcome.within_duration_limit()); // duration passed
        assert!(!outcome.is_success()); // overall failure
    }

    #[test]
    fn both_postcondition_and_duration_fail() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure("always fails", |_input, _response| {
                Err(ContractViolation::new("check", "forced"))
            })
            .ensure_duration_below(Duration::from_millis(100))
            .build();

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(200));

        assert!(outcome.violation().is_some());
        assert!(!outcome.within_duration_limit());
        assert!(!outcome.is_success());
    }

    #[test]
    #[should_panic(expected = "Contract violations")]
    fn assert_contract_panics_on_duration_violation() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure_duration_below(Duration::from_millis(100))
            .build();

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(500));
        outcome.assert_contract();
    }

    #[test]
    #[should_panic(expected = "Postcondition 'check' failed: forced")]
    fn assert_contract_reports_postcondition_before_duration() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure("always fails", |_input, _response| {
                Err(ContractViolation::new("check", "forced"))
            })
            .ensure_duration_below(Duration::from_millis(100))
            .build();

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(500));
        outcome.assert_contract(); // should report postcondition first
    }

    #[test]
    fn from_response_evaluates_duration_constraint() {
        let contract = ServiceContract::<u32, u32>::builder()
            .ensure_duration_below(Duration::from_millis(500))
            .build();

        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(300));
        assert!(outcome.duration_result().is_some());
        assert!(outcome.within_duration_limit());
    }

    // --- Conformance tests ---

    #[test]
    fn no_conformance_means_matches_expected_is_true() {
        let contract = ServiceContract::<u32, u32>::builder().build();
        let outcome = ServiceContractOutcome::from_response(&contract, &1, 42, Duration::from_millis(10));
        assert!(outcome.matches_expected());
        assert!(outcome.conformance_result().is_none());
    }

    #[test]
    fn conforms_to_match_succeeds() {
        use crate::contract::conformance::StringMatcher;

        let contract = ServiceContract::<String, String>::builder().build();
        let outcome = ServiceContractOutcome::from_response(
            &contract,
            &"input".into(),
            "hello".into(),
            Duration::from_millis(10),
        );
        let outcome =
            outcome.conforms_to(&"hello".to_string(), |r| r.clone(), &StringMatcher::exact());

        assert!(outcome.matches_expected());
        assert!(outcome.conformance_result().unwrap().is_match());
        assert!(outcome.is_success());
    }

    #[test]
    fn conforms_to_mismatch_fails() {
        use crate::contract::conformance::StringMatcher;

        let contract = ServiceContract::<String, String>::builder().build();
        let outcome = ServiceContractOutcome::from_response(
            &contract,
            &"input".into(),
            "hello".into(),
            Duration::from_millis(10),
        );
        let outcome = outcome.conforms_to(
            &"goodbye".to_string(),
            |r| r.clone(),
            &StringMatcher::exact(),
        );

        assert!(!outcome.matches_expected());
        assert!(!outcome.is_success());
        let cr = outcome.conformance_result().unwrap();
        assert!(cr.match_result().is_mismatch());
        assert!(cr.match_result().diff().contains("hello"));
        assert!(cr.match_result().diff().contains("goodbye"));
    }

    #[test]
    fn conforms_to_extractor_projects_response() {
        use crate::contract::conformance::StringMatcher;

        let contract = ServiceContract::<String, String>::builder().build();
        let outcome = ServiceContractOutcome::from_response(
            &contract,
            &"input".into(),
            "HELLO WORLD".into(),
            Duration::from_millis(10),
        );
        // Extract first word
        let outcome = outcome.conforms_to(
            &"HELLO".to_string(),
            |r| r.split_whitespace().next().unwrap_or("").to_string(),
            &StringMatcher::exact(),
        );

        assert!(outcome.matches_expected());
    }

    #[test]
    fn postcondition_pass_conformance_fail_means_overall_failure() {
        use crate::contract::conformance::StringMatcher;

        let contract = ServiceContract::<String, String>::builder()
            .ensure("not empty", |_input, response| {
                if response.is_empty() {
                    Err(ContractViolation::new("content", "empty"))
                } else {
                    Ok(())
                }
            })
            .build();

        let outcome = ServiceContractOutcome::from_response(
            &contract,
            &"input".into(),
            "hello".into(),
            Duration::from_millis(10),
        );
        let outcome = outcome.conforms_to(
            &"goodbye".to_string(),
            |r| r.clone(),
            &StringMatcher::exact(),
        );

        assert!(outcome.violation().is_none()); // postconditions passed
        assert!(!outcome.matches_expected()); // conformance failed
        assert!(!outcome.is_success()); // overall failure
    }

    #[test]
    fn all_three_dimensions_pass() {
        use crate::contract::conformance::StringMatcher;

        let contract = ServiceContract::<String, String>::builder()
            .ensure("not empty", |_input, response| {
                if response.is_empty() {
                    Err(ContractViolation::new("content", "empty"))
                } else {
                    Ok(())
                }
            })
            .ensure_duration_below(Duration::from_millis(500))
            .build();

        let outcome = ServiceContractOutcome::from_response(
            &contract,
            &"input".into(),
            "hello".into(),
            Duration::from_millis(100),
        );
        let outcome =
            outcome.conforms_to(&"hello".to_string(), |r| r.clone(), &StringMatcher::exact());

        assert!(outcome.violation().is_none());
        assert!(outcome.within_duration_limit());
        assert!(outcome.matches_expected());
        assert!(outcome.is_success());
    }

    #[test]
    #[should_panic(expected = "Expected value mismatch")]
    fn assert_contract_includes_conformance_mismatch() {
        use crate::contract::conformance::StringMatcher;

        let contract = ServiceContract::<String, String>::builder().build();
        let outcome = ServiceContractOutcome::from_response(
            &contract,
            &"input".into(),
            "hello".into(),
            Duration::from_millis(10),
        );
        let outcome = outcome.conforms_to(
            &"goodbye".to_string(),
            |r| r.clone(),
            &StringMatcher::exact(),
        );
        outcome.assert_contract();
    }
}
