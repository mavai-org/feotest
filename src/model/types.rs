//! Shared domain types used across the framework.

use std::fmt;
use std::time::Duration;

/// The intent behind a probabilistic test.
///
/// Determines how the framework enforces statistical feasibility and
/// qualifies the resulting verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestIntent {
    /// Evidential claim. The framework rejects the configuration before
    /// execution if sample size cannot support verification at 95% confidence
    /// (when threshold origin is normative).
    Verification,

    /// Lightweight early-warning check. Accepts undersized configurations
    /// but labels the verdict as non-evidential.
    Smoke,
}

impl fmt::Display for TestIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verification => write!(f, "VERIFICATION"),
            Self::Smoke => write!(f, "SMOKE"),
        }
    }
}

/// The provenance of a pass-rate threshold.
///
/// Documents where a threshold comes from and whether it is normative
/// (carries enforcement consequences under `Verification` intent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdOrigin {
    /// Externally agreed normative target (contract/SLA).
    Sla,
    /// Internally defined normative target (service objective).
    Slo,
    /// Normative target derived from policy or governance.
    Policy,
    /// Empirical reference value derived from a baseline measurement.
    Empirical,
    /// No provenance specified. Non-normative.
    Unspecified,
}

impl ThresholdOrigin {
    /// Whether this origin carries normative enforcement consequences.
    ///
    /// Normative origins (`Sla`, `Slo`, `Policy`) trigger feasibility
    /// enforcement under `Verification` intent.
    #[must_use]
    pub const fn is_normative(self) -> bool {
        matches!(self, Self::Sla | Self::Slo | Self::Policy)
    }
}

impl fmt::Display for ThresholdOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sla => write!(f, "SLA"),
            Self::Slo => write!(f, "SLO"),
            Self::Policy => write!(f, "POLICY"),
            Self::Empirical => write!(f, "EMPIRICAL"),
            Self::Unspecified => write!(f, "UNSPECIFIED"),
        }
    }
}

/// What to do when a budget (time or tokens) is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetExhaustedBehavior {
    /// Fail the test immediately.
    Fail,
    /// Evaluate whatever samples have been collected so far.
    EvaluatePartial,
}

/// How the execution engine treats panics or errors during trial execution.
///
/// Note: per design decision, panics in trial closures are *not* caught.
/// A panic is a defect, not a contract violation. This enum exists for
/// future extensibility but currently only the `Abort` variant is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionHandling {
    /// Abort the entire test run immediately.
    Abort,
}

/// Why execution terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationReason {
    /// All planned samples were executed.
    Completed,
    /// Time budget exhausted.
    TimeBudgetExhausted,
    /// Token budget exhausted.
    TokenBudgetExhausted,
    /// Early termination: failure is inevitable.
    FailureInevitable,
    /// Early termination: success is guaranteed.
    SuccessGuaranteed,
}

impl fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed => write!(f, "COMPLETED"),
            Self::TimeBudgetExhausted => write!(f, "TIME_BUDGET_EXHAUSTED"),
            Self::TokenBudgetExhausted => write!(f, "TOKEN_BUDGET_EXHAUSTED"),
            Self::FailureInevitable => write!(f, "FAILURE_INEVITABLE"),
            Self::SuccessGuaranteed => write!(f, "SUCCESS_GUARANTEED"),
        }
    }
}

/// Information about why and how execution terminated.
#[derive(Debug, Clone)]
pub struct TerminationInfo {
    reason: TerminationReason,
    detail: Option<String>,
}

impl TerminationInfo {
    /// Creates termination info with a reason and optional detail.
    #[must_use]
    pub const fn new(reason: TerminationReason) -> Self {
        Self {
            reason,
            detail: None,
        }
    }

    /// Adds a detail message.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The termination reason.
    #[must_use]
    pub const fn reason(&self) -> &TerminationReason {
        &self.reason
    }

    /// Optional detail about the termination.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

/// Identifies a specific test or experiment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestIdentity {
    use_case_id: String,
    test_name: Option<String>,
}

impl TestIdentity {
    /// Creates a test identity from a use case ID.
    #[must_use]
    pub fn new(use_case_id: impl Into<String>) -> Self {
        Self {
            use_case_id: use_case_id.into(),
            test_name: None,
        }
    }

    /// Adds a test name to the identity.
    #[must_use]
    pub fn with_test_name(mut self, name: impl Into<String>) -> Self {
        self.test_name = Some(name.into());
        self
    }

    /// The use case identifier.
    #[must_use]
    pub fn use_case_id(&self) -> &str {
        &self.use_case_id
    }

    /// The test name, if any.
    #[must_use]
    pub fn test_name(&self) -> Option<&str> {
        self.test_name.as_deref()
    }
}

/// Summary of execution costs.
#[derive(Debug, Clone)]
pub struct CostSummary {
    total_time: Duration,
    total_tokens: u64,
    samples_executed: u32,
}

impl CostSummary {
    /// Creates a cost summary.
    #[must_use]
    pub const fn new(total_time: Duration, total_tokens: u64, samples_executed: u32) -> Self {
        Self {
            total_time,
            total_tokens,
            samples_executed,
        }
    }

    /// Total wall-clock time for all samples.
    #[must_use]
    pub const fn total_time(&self) -> Duration {
        self.total_time
    }

    /// Average time per sample, or zero if none executed.
    #[must_use]
    pub fn avg_time_per_sample(&self) -> Duration {
        if self.samples_executed == 0 {
            Duration::ZERO
        } else {
            self.total_time / self.samples_executed
        }
    }

    /// Total tokens consumed across all samples.
    #[must_use]
    pub const fn total_tokens(&self) -> u64 {
        self.total_tokens
    }

    /// Average tokens per sample, or 0 if none executed.
    #[must_use]
    pub fn avg_tokens_per_sample(&self) -> u64 {
        if self.samples_executed == 0 {
            0
        } else {
            self.total_tokens / u64::from(self.samples_executed)
        }
    }

    /// Number of samples executed.
    #[must_use]
    pub const fn samples_executed(&self) -> u32 {
        self.samples_executed
    }
}

/// Summary of a completed execution run.
#[derive(Debug, Clone)]
pub struct ExecutionSummary {
    samples_planned: u32,
    samples_executed: u32,
    successes: u32,
    failures: u32,
    termination: TerminationInfo,
    cost: CostSummary,
}

impl ExecutionSummary {
    /// Creates an execution summary.
    #[must_use]
    pub const fn new(
        samples_planned: u32,
        samples_executed: u32,
        successes: u32,
        failures: u32,
        termination: TerminationInfo,
        cost: CostSummary,
    ) -> Self {
        Self {
            samples_planned,
            samples_executed,
            successes,
            failures,
            termination,
            cost,
        }
    }

    /// Number of samples originally planned.
    #[must_use]
    pub const fn samples_planned(&self) -> u32 {
        self.samples_planned
    }

    /// Number of samples actually executed.
    #[must_use]
    pub const fn samples_executed(&self) -> u32 {
        self.samples_executed
    }

    /// Number of successful trials.
    #[must_use]
    pub const fn successes(&self) -> u32 {
        self.successes
    }

    /// Number of failed trials.
    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// Observed pass rate.
    #[must_use]
    pub fn observed_pass_rate(&self) -> f64 {
        if self.samples_executed == 0 {
            0.0
        } else {
            f64::from(self.successes) / f64::from(self.samples_executed)
        }
    }

    /// How and why execution terminated.
    #[must_use]
    pub const fn termination(&self) -> &TerminationInfo {
        &self.termination
    }

    /// Cost summary for the execution.
    #[must_use]
    pub const fn cost(&self) -> &CostSummary {
        &self.cost
    }
}

/// A warning attached to a verdict or execution result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    code: String,
    message: String,
}

impl Warning {
    /// Creates a new warning.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Warning code for programmatic handling.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Human-readable warning message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normative_threshold_origins() {
        assert!(ThresholdOrigin::Sla.is_normative());
        assert!(ThresholdOrigin::Slo.is_normative());
        assert!(ThresholdOrigin::Policy.is_normative());
        assert!(!ThresholdOrigin::Empirical.is_normative());
        assert!(!ThresholdOrigin::Unspecified.is_normative());
    }

    #[test]
    fn test_identity_with_and_without_test_name() {
        let id = TestIdentity::new("shopping-basket");
        assert_eq!(id.use_case_id(), "shopping-basket");
        assert!(id.test_name().is_none());

        let id = id.with_test_name("test_translation");
        assert_eq!(id.test_name(), Some("test_translation"));
    }

    #[test]
    fn cost_summary_averages() {
        let cost = CostSummary::new(Duration::from_millis(1000), 500, 10);
        assert_eq!(cost.avg_time_per_sample(), Duration::from_millis(100));
        assert_eq!(cost.avg_tokens_per_sample(), 50);
    }

    #[test]
    fn cost_summary_zero_samples() {
        let cost = CostSummary::new(Duration::ZERO, 0, 0);
        assert_eq!(cost.avg_time_per_sample(), Duration::ZERO);
        assert_eq!(cost.avg_tokens_per_sample(), 0);
    }

    #[test]
    fn execution_summary_observed_pass_rate() {
        let term = TerminationInfo::new(TerminationReason::Completed);
        let cost = CostSummary::new(Duration::from_millis(100), 0, 10);
        let summary = ExecutionSummary::new(10, 10, 8, 2, term, cost);
        assert!((summary.observed_pass_rate() - 0.8).abs() < 1e-10);
    }

    #[test]
    fn warning_displays_code_and_message() {
        let w = Warning::new("BASELINE_EXPIRED", "Baseline is 45 days old");
        assert_eq!(w.to_string(), "[BASELINE_EXPIRED] Baseline is 45 days old");
    }
}
