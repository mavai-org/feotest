//! Shared domain types used across the framework.

use std::fmt;
use std::time::Duration;

use crate::controls::PacingConfig;

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
    /// Method-level time budget exhausted.
    TimeBudgetExhausted,
    /// Method-level token budget exhausted.
    TokenBudgetExhausted,
    /// Run-scoped time budget exhausted. The shared wall-clock cap for
    /// the cargo invocation ran out.
    RunTimeBudgetExhausted,
    /// Run-scoped token budget exhausted. The shared token cap for the
    /// cargo invocation ran out.
    RunTokenBudgetExhausted,
    /// Early termination: failure is inevitable.
    FailureInevitable,
    /// Early termination: success is guaranteed.
    SuccessGuaranteed,
}

impl TerminationReason {
    /// Whether this reason denotes any kind of budget exhaustion
    /// (method-level or run-scoped).
    #[must_use]
    pub const fn is_budget_exhausted(&self) -> bool {
        matches!(
            self,
            Self::TimeBudgetExhausted
                | Self::TokenBudgetExhausted
                | Self::RunTimeBudgetExhausted
                | Self::RunTokenBudgetExhausted
        )
    }
}

impl fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Completed => write!(f, "COMPLETED"),
            Self::TimeBudgetExhausted => write!(f, "TIME_BUDGET_EXHAUSTED"),
            Self::TokenBudgetExhausted => write!(f, "TOKEN_BUDGET_EXHAUSTED"),
            Self::RunTimeBudgetExhausted => write!(f, "RUN_TIME_BUDGET_EXHAUSTED"),
            Self::RunTokenBudgetExhausted => write!(f, "RUN_TOKEN_BUDGET_EXHAUSTED"),
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

/// Snapshot of the run-scoped budget at the moment a test terminated.
///
/// Captures what the shared process-wide budget looked like at the
/// instant the engine stopped sampling. Used by the warning dispatch
/// to render "consumed X of Y" for the run-scoped exhaustion variants.
///
/// Present on [`CostSummary`] only when the enclosing test ran with a
/// run-scoped budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunScopedSnapshot {
    time_budget: Option<Duration>,
    time_consumed: Duration,
    token_budget: Option<u64>,
    tokens_consumed: u64,
}

impl RunScopedSnapshot {
    /// Creates a snapshot.
    #[must_use]
    pub const fn new(
        time_budget: Option<Duration>,
        time_consumed: Duration,
        token_budget: Option<u64>,
        tokens_consumed: u64,
    ) -> Self {
        Self {
            time_budget,
            time_consumed,
            token_budget,
            tokens_consumed,
        }
    }

    /// Configured run-scoped time cap, if any.
    #[must_use]
    pub const fn time_budget(&self) -> Option<Duration> {
        self.time_budget
    }

    /// Wall-clock time consumed against the run-scoped budget so far.
    #[must_use]
    pub const fn time_consumed(&self) -> Duration {
        self.time_consumed
    }

    /// Configured run-scoped token cap, if any.
    #[must_use]
    pub const fn token_budget(&self) -> Option<u64> {
        self.token_budget
    }

    /// Tokens consumed against the run-scoped budget so far.
    #[must_use]
    pub const fn tokens_consumed(&self) -> u64 {
        self.tokens_consumed
    }
}

/// Summary of execution costs.
#[derive(Debug, Clone)]
pub struct CostSummary {
    total_time: Duration,
    total_tokens: u64,
    samples_executed: u32,
    run_scoped: Option<RunScopedSnapshot>,
}

impl CostSummary {
    /// Creates a cost summary with no run-scoped snapshot.
    #[must_use]
    pub const fn new(total_time: Duration, total_tokens: u64, samples_executed: u32) -> Self {
        Self {
            total_time,
            total_tokens,
            samples_executed,
            run_scoped: None,
        }
    }

    /// Attaches a run-scoped budget snapshot captured at termination.
    #[must_use]
    pub const fn with_run_scoped(mut self, snapshot: RunScopedSnapshot) -> Self {
        self.run_scoped = Some(snapshot);
        self
    }

    /// The run-scoped budget snapshot, if the test ran under a
    /// run-scoped budget.
    #[must_use]
    pub const fn run_scoped(&self) -> Option<&RunScopedSnapshot> {
        self.run_scoped.as_ref()
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

/// Resolved pacing constraints recorded on a verdict.
///
/// Captures both the configured limits and the effective values after
/// constraint resolution. All fields reflect the state at verdict time,
/// not the configuration input.
#[derive(Debug, Clone)]
pub struct PacingSummary {
    max_rps: f64,
    max_rpm: f64,
    max_concurrent: u32,
    effective_min_delay_ms: u64,
    effective_concurrency: u32,
    effective_rps: f64,
}

impl PacingSummary {
    /// Constructs a pacing summary from a resolved `PacingConfig`.
    ///
    /// Computes effective values from the most-restrictive constraint.
    #[must_use]
    pub fn from_config(config: &PacingConfig) -> Self {
        let effective_delay = config.effective_delay_ms();
        let effective_rps = if effective_delay > 0 {
            1000.0 / effective_delay as f64
        } else {
            f64::INFINITY
        };
        Self {
            max_rps: config.configured_max_requests_per_second().unwrap_or(0.0),
            max_rpm: config.configured_max_requests_per_minute().unwrap_or(0.0),
            max_concurrent: 1,
            effective_min_delay_ms: effective_delay,
            effective_concurrency: 1,
            effective_rps,
        }
    }

    /// Configured maximum requests per second, or 0 if unconstrained.
    #[must_use]
    pub const fn max_rps(&self) -> f64 {
        self.max_rps
    }

    /// Configured maximum requests per minute, or 0 if unconstrained.
    #[must_use]
    pub const fn max_rpm(&self) -> f64 {
        self.max_rpm
    }

    /// Maximum concurrent requests (always 1 until RC11).
    #[must_use]
    pub const fn max_concurrent(&self) -> u32 {
        self.max_concurrent
    }

    /// Effective minimum delay between samples in milliseconds.
    #[must_use]
    pub const fn effective_min_delay_ms(&self) -> u64 {
        self.effective_min_delay_ms
    }

    /// Effective concurrency level (always 1 until RC11).
    #[must_use]
    pub const fn effective_concurrency(&self) -> u32 {
        self.effective_concurrency
    }

    /// Effective requests per second after constraint resolution.
    #[must_use]
    pub const fn effective_rps(&self) -> f64 {
        self.effective_rps
    }
}

/// Baseline freshness status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpirationStatus {
    /// No expiration policy defined.
    NoExpiration,
    /// Baseline is within its validity period.
    Valid,
    /// Baseline is approaching expiration.
    ExpiringSoon,
    /// Baseline is very close to expiration.
    ExpiringImminently,
    /// Baseline has expired.
    Expired,
}

impl ExpirationStatus {
    /// Whether this status warrants a warning in reports.
    #[must_use]
    pub const fn requires_warning(&self) -> bool {
        matches!(
            self,
            Self::ExpiringSoon | Self::ExpiringImminently | Self::Expired
        )
    }

    /// The XML-safe name for this status.
    #[must_use]
    pub const fn xml_name(&self) -> &str {
        match self {
            Self::NoExpiration => "NO_EXPIRATION",
            Self::Valid => "VALID",
            Self::ExpiringSoon => "EXPIRING_SOON",
            Self::ExpiringImminently => "EXPIRING_IMMINENTLY",
            Self::Expired => "EXPIRED",
        }
    }
}

/// Expiration information for a baseline spec.
#[derive(Debug, Clone)]
pub struct ExpirationInfo {
    status: ExpirationStatus,
    expires_at: Option<String>,
}

impl ExpirationInfo {
    /// Creates expiration info with a status and optional expiry timestamp.
    #[must_use]
    pub const fn new(status: ExpirationStatus, expires_at: Option<String>) -> Self {
        Self { status, expires_at }
    }

    /// The expiration status.
    #[must_use]
    pub const fn status(&self) -> &ExpirationStatus {
        &self.status
    }

    /// ISO 8601 timestamp when the baseline expires, if known.
    #[must_use]
    pub fn expires_at(&self) -> Option<&str> {
        self.expires_at.as_deref()
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

    #[test]
    fn pacing_summary_from_config() {
        let config = PacingConfig::new()
            .max_requests_per_second(5.0)
            .max_requests_per_minute(120.0);
        let summary = PacingSummary::from_config(&config);

        assert!((summary.max_rps() - 5.0).abs() < 1e-10);
        assert!((summary.max_rpm() - 120.0).abs() < 1e-10);
        assert_eq!(summary.max_concurrent(), 1);
        assert_eq!(summary.effective_concurrency(), 1);
        // 5 rps → 200ms delay; 120 rpm → 500ms delay; most restrictive = 500ms
        assert_eq!(summary.effective_min_delay_ms(), 500);
        assert!((summary.effective_rps() - 2.0).abs() < 1e-10);
    }

    #[test]
    fn pacing_summary_unconstrained() {
        let config = PacingConfig::new();
        let summary = PacingSummary::from_config(&config);

        assert!((summary.max_rps()).abs() < 1e-10);
        assert!((summary.max_rpm()).abs() < 1e-10);
        assert_eq!(summary.effective_min_delay_ms(), 0);
        assert!(summary.effective_rps().is_infinite());
    }

    #[test]
    fn expiration_status_requires_warning() {
        assert!(!ExpirationStatus::NoExpiration.requires_warning());
        assert!(!ExpirationStatus::Valid.requires_warning());
        assert!(ExpirationStatus::ExpiringSoon.requires_warning());
        assert!(ExpirationStatus::ExpiringImminently.requires_warning());
        assert!(ExpirationStatus::Expired.requires_warning());
    }

    #[test]
    fn expiration_info_accessors() {
        let info = ExpirationInfo::new(
            ExpirationStatus::Expired,
            Some("2026-05-01T00:00:00Z".into()),
        );
        assert_eq!(info.status(), &ExpirationStatus::Expired);
        assert_eq!(info.expires_at(), Some("2026-05-01T00:00:00Z"));

        let info_none = ExpirationInfo::new(ExpirationStatus::NoExpiration, None);
        assert!(info_none.expires_at().is_none());
    }
}
