//! Builder for constructing service contracts.

use std::time::Duration;

use crate::contract::{Check, DurationConstraint, ServiceContract};
use crate::model::Outcome;

/// Builder for [`ServiceContract`].
///
/// Collects postcondition checks in declaration order, and an optional
/// duration constraint.
pub struct ServiceContractBuilder<I, R> {
    checks: Vec<Check<I, R>>,
    duration_constraint: Option<DurationConstraint>,
}

impl<I, R> ServiceContractBuilder<I, R> {
    pub(crate) const fn new() -> Self {
        Self {
            checks: Vec::new(),
            duration_constraint: None,
        }
    }

    /// Adds a named postcondition check.
    ///
    /// The check closure receives references to the input and response,
    /// and returns `Ok(())` on success or `Err(ContractViolation)` on failure.
    /// Checks are evaluated in declaration order; the first failure short-circuits.
    ///
    /// # Examples
    ///
    /// ```
    /// use feotest::contract::ServiceContract;
    /// use feotest::model::ContractViolation;
    ///
    /// let contract = ServiceContract::<String, String>::builder()
    ///     .ensure("Response has content", |_input, response| {
    ///         if response.is_empty() {
    ///             Err(ContractViolation::new("content", "empty response"))
    ///         } else {
    ///             Ok(())
    ///         }
    ///     })
    ///     .ensure("Response is JSON", |_input, response| {
    ///         if response.starts_with('{') {
    ///             Ok(())
    ///         } else {
    ///             Err(ContractViolation::new("format", "not JSON"))
    ///         }
    ///     })
    ///     .build();
    ///
    /// assert!(contract.evaluate(&"input".into(), &"{\"ok\":true}".into()).is_ok());
    /// assert!(contract.evaluate(&"input".into(), &String::new()).is_err());
    /// ```
    #[must_use]
    pub fn ensure(
        mut self,
        name: impl Into<String>,
        check: impl Fn(&I, &R) -> Outcome + Send + Sync + 'static,
    ) -> Self {
        self.checks.push(Check {
            name: name.into(),
            f: Box::new(check),
        });
        self
    }

    /// Adds a duration constraint with a default description.
    ///
    /// The constraint is evaluated independently from postcondition checks.
    /// Both dimensions (correctness and timing) are always assessed,
    /// regardless of whether one or both fail.
    ///
    /// Only one duration constraint may be set; calling this again replaces
    /// any previously set constraint.
    ///
    /// # Panics
    ///
    /// Panics if `max_duration` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use feotest::contract::ServiceContract;
    ///
    /// let contract = ServiceContract::<String, String>::builder()
    ///     .ensure_duration_below(Duration::from_millis(500))
    ///     .build();
    ///
    /// assert!(contract.duration_constraint().is_some());
    /// ```
    #[must_use]
    pub fn ensure_duration_below(mut self, max_duration: Duration) -> Self {
        self.duration_constraint = Some(DurationConstraint::below(max_duration));
        self
    }

    /// Adds a duration constraint with a custom description.
    ///
    /// The constraint is evaluated independently from postcondition checks.
    /// Both dimensions (correctness and timing) are always assessed,
    /// regardless of whether one or both fail.
    ///
    /// Only one duration constraint may be set; calling this again replaces
    /// any previously set constraint.
    ///
    /// # Panics
    ///
    /// Panics if `max_duration` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use feotest::contract::ServiceContract;
    ///
    /// let contract = ServiceContract::<String, String>::builder()
    ///     .ensure_duration_below_with_description(
    ///         "API response time",
    ///         Duration::from_secs(2),
    ///     )
    ///     .build();
    ///
    /// let constraint = contract.duration_constraint().unwrap();
    /// assert_eq!(constraint.description(), "API response time");
    /// ```
    #[must_use]
    pub fn ensure_duration_below_with_description(
        mut self,
        description: impl Into<String>,
        max_duration: Duration,
    ) -> Self {
        self.duration_constraint = Some(DurationConstraint::below_with_description(
            description,
            max_duration,
        ));
        self
    }

    /// Builds the service contract.
    #[must_use]
    pub fn build(self) -> ServiceContract<I, R> {
        ServiceContract {
            checks: self.checks,
            duration_constraint: self.duration_constraint,
        }
    }
}
