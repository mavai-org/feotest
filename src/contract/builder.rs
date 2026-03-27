//! Builder for constructing service contracts.

use crate::contract::{Check, ServiceContract};
use crate::model::Outcome;

/// Builder for [`ServiceContract`].
///
/// Collects postcondition checks in declaration order.
pub struct ServiceContractBuilder<I, R> {
    checks: Vec<Check<I, R>>,
}

impl<I, R> ServiceContractBuilder<I, R> {
    pub(crate) const fn new() -> Self {
        Self { checks: Vec::new() }
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

    /// Builds the service contract.
    #[must_use]
    pub fn build(self) -> ServiceContract<I, R> {
        ServiceContract {
            checks: self.checks,
        }
    }
}
