//! Execution configuration types.

use std::time::Duration;

use crate::model::BudgetExhaustedBehavior;

/// Configuration governing execution of trials.
///
/// Captures all parameters that the execution engine needs:
/// sample count, warmup, budgets, pacing, and failure handling.
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    samples: u32,
    warmup: u32,
    time_budget: Option<Duration>,
    token_budget: Option<u64>,
    static_token_charge: Option<u64>,
    on_budget_exhausted: BudgetExhaustedBehavior,
    max_example_failures: u32,
    pacing: Option<PacingConfig>,
}

impl ExecutionConfig {
    /// Creates an execution config with the given sample count.
    ///
    /// All other fields use sensible defaults:
    /// - No warmup
    /// - No budgets
    /// - Fail on budget exhaustion
    /// - Up to 5 example failures captured
    /// - No pacing
    /// # Panics
    ///
    /// Panics if `samples` is zero.
    #[must_use]
    pub fn new(samples: u32) -> Self {
        assert!(samples > 0, "sample count must be positive");
        Self {
            samples,
            warmup: 0,
            time_budget: None,
            token_budget: None,
            static_token_charge: None,
            on_budget_exhausted: BudgetExhaustedBehavior::Fail,
            max_example_failures: 5,
            pacing: None,
        }
    }

    /// Sets the warmup count.
    #[must_use]
    pub const fn with_warmup(mut self, warmup: u32) -> Self {
        self.warmup = warmup;
        self
    }

    /// Sets the time budget.
    #[must_use]
    pub const fn with_time_budget(mut self, budget: Duration) -> Self {
        self.time_budget = Some(budget);
        self
    }

    /// Sets the token budget.
    #[must_use]
    pub const fn with_token_budget(mut self, budget: u64) -> Self {
        self.token_budget = Some(budget);
        self
    }

    /// Sets a static token charge per sample.
    #[must_use]
    pub const fn with_static_token_charge(mut self, charge: u64) -> Self {
        self.static_token_charge = Some(charge);
        self
    }

    /// Sets the behaviour when a budget is exhausted.
    #[must_use]
    pub const fn with_on_budget_exhausted(mut self, behaviour: BudgetExhaustedBehavior) -> Self {
        self.on_budget_exhausted = behaviour;
        self
    }

    /// Sets the maximum number of example failures to capture.
    #[must_use]
    pub const fn with_max_example_failures(mut self, max: u32) -> Self {
        self.max_example_failures = max;
        self
    }

    /// Sets pacing constraints.
    #[must_use]
    pub const fn with_pacing(mut self, pacing: PacingConfig) -> Self {
        self.pacing = Some(pacing);
        self
    }

    // --- Internal helpers for experiment builders ---

    /// Sets the time budget on this config (consuming and returning it).
    #[must_use]
    pub(crate) const fn set_time_budget(mut self, budget: Duration) -> Self {
        self.time_budget = Some(budget);
        self
    }

    /// Sets the token budget on this config (consuming and returning it).
    #[must_use]
    pub(crate) const fn set_token_budget(mut self, budget: u64) -> Self {
        self.token_budget = Some(budget);
        self
    }

    /// Sets the pacing config (consuming and returning it).
    #[must_use]
    pub(crate) const fn set_pacing(mut self, pacing: PacingConfig) -> Self {
        self.pacing = Some(pacing);
        self
    }

    /// Number of samples to execute.
    #[must_use]
    pub const fn samples(&self) -> u32 {
        self.samples
    }

    /// Number of warmup invocations to discard.
    #[must_use]
    pub const fn warmup(&self) -> u32 {
        self.warmup
    }

    /// Time budget, if set.
    #[must_use]
    pub const fn time_budget(&self) -> Option<Duration> {
        self.time_budget
    }

    /// Token budget, if set.
    #[must_use]
    pub const fn token_budget(&self) -> Option<u64> {
        self.token_budget
    }

    /// Static token charge per sample, if set.
    #[must_use]
    pub const fn static_token_charge(&self) -> Option<u64> {
        self.static_token_charge
    }

    /// What to do when a budget is exhausted.
    #[must_use]
    pub const fn on_budget_exhausted(&self) -> BudgetExhaustedBehavior {
        self.on_budget_exhausted
    }

    /// Maximum number of example failures to capture.
    #[must_use]
    pub const fn max_example_failures(&self) -> u32 {
        self.max_example_failures
    }

    /// Pacing configuration, if set.
    #[must_use]
    pub const fn pacing(&self) -> Option<&PacingConfig> {
        self.pacing.as_ref()
    }
}

/// Pacing constraints for rate-limiting trial execution.
///
/// Multiple constraints can be set; the most restrictive one wins.
#[derive(Debug, Clone)]
pub struct PacingConfig {
    min_ms_per_sample: Option<u64>,
    max_requests_per_second: Option<f64>,
    max_requests_per_minute: Option<f64>,
}

impl PacingConfig {
    /// Creates a pacing config with no constraints.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            min_ms_per_sample: None,
            max_requests_per_second: None,
            max_requests_per_minute: None,
        }
    }

    /// Sets the minimum milliseconds between samples.
    #[must_use]
    pub const fn with_min_ms_per_sample(mut self, ms: u64) -> Self {
        self.min_ms_per_sample = Some(ms);
        self
    }

    /// Sets the maximum requests per second.
    ///
    /// # Panics
    ///
    /// Panics if `rps` is not positive.
    #[must_use]
    pub fn with_max_requests_per_second(mut self, rps: f64) -> Self {
        assert!(rps > 0.0, "max requests per second must be positive");
        self.max_requests_per_second = Some(rps);
        self
    }

    /// Sets the maximum requests per minute.
    ///
    /// # Panics
    ///
    /// Panics if `rpm` is not positive.
    #[must_use]
    pub fn with_max_requests_per_minute(mut self, rpm: f64) -> Self {
        assert!(rpm > 0.0, "max requests per minute must be positive");
        self.max_requests_per_minute = Some(rpm);
        self
    }

    /// Computes the effective minimum delay between samples in milliseconds.
    ///
    /// Takes the most restrictive of all configured constraints.
    #[must_use]
    pub fn effective_delay_ms(&self) -> u64 {
        let mut delay = 0u64;

        if let Some(ms) = self.min_ms_per_sample {
            delay = delay.max(ms);
        }

        if let Some(rps) = self.max_requests_per_second {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let ms = (1000.0 / rps).ceil() as u64;
            delay = delay.max(ms);
        }

        if let Some(rpm) = self.max_requests_per_minute {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let ms = (60_000.0 / rpm).ceil() as u64;
            delay = delay.max(ms);
        }

        delay
    }
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_config_defaults() {
        let config = ExecutionConfig::new(100);
        assert_eq!(config.samples(), 100);
        assert_eq!(config.warmup(), 0);
        assert!(config.time_budget().is_none());
        assert!(config.token_budget().is_none());
        assert_eq!(config.on_budget_exhausted(), BudgetExhaustedBehavior::Fail);
        assert_eq!(config.max_example_failures(), 5);
        assert!(config.pacing().is_none());
    }

    #[test]
    fn execution_config_builder_methods() {
        let config = ExecutionConfig::new(200)
            .with_warmup(10)
            .with_time_budget(Duration::from_secs(60))
            .with_token_budget(100_000)
            .with_static_token_charge(150)
            .with_on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial)
            .with_max_example_failures(3);

        assert_eq!(config.samples(), 200);
        assert_eq!(config.warmup(), 10);
        assert_eq!(config.time_budget(), Some(Duration::from_secs(60)));
        assert_eq!(config.token_budget(), Some(100_000));
        assert_eq!(config.static_token_charge(), Some(150));
        assert_eq!(
            config.on_budget_exhausted(),
            BudgetExhaustedBehavior::EvaluatePartial
        );
        assert_eq!(config.max_example_failures(), 3);
    }

    #[test]
    #[should_panic(expected = "sample count must be positive")]
    fn rejects_zero_samples() {
        ExecutionConfig::new(0);
    }

    #[test]
    fn pacing_no_constraints() {
        let pacing = PacingConfig::new();
        assert_eq!(pacing.effective_delay_ms(), 0);
    }

    #[test]
    fn pacing_min_ms_per_sample() {
        let pacing = PacingConfig::new().with_min_ms_per_sample(200);
        assert_eq!(pacing.effective_delay_ms(), 200);
    }

    #[test]
    fn pacing_rps_constraint() {
        let pacing = PacingConfig::new().with_max_requests_per_second(5.0);
        assert_eq!(pacing.effective_delay_ms(), 200);
    }

    #[test]
    fn pacing_rpm_constraint() {
        let pacing = PacingConfig::new().with_max_requests_per_minute(60.0);
        assert_eq!(pacing.effective_delay_ms(), 1000);
    }

    #[test]
    fn pacing_most_restrictive_wins() {
        let pacing = PacingConfig::new()
            .with_min_ms_per_sample(100)
            .with_max_requests_per_second(2.0); // 500ms
        assert_eq!(pacing.effective_delay_ms(), 500);
    }
}
