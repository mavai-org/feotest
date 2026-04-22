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
    min_pass_rate: Option<f64>,
    min_samples_for_validity: Option<u32>,
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
            min_pass_rate: None,
            min_samples_for_validity: None,
        }
    }

    /// Sets the warmup count.
    #[must_use]
    pub const fn with_warmup(mut self, warmup: u32) -> Self {
        self.warmup = warmup;
        self
    }

    /// Sets the time budget.
    ///
    /// # Panics
    ///
    /// Panics if `budget` is zero. A non-positive budget has no meaningful
    /// behaviour — no sample could ever run.
    #[must_use]
    pub fn with_time_budget(mut self, budget: Duration) -> Self {
        assert!(
            !budget.is_zero(),
            "time_budget must be positive, got {budget:?}"
        );
        self.time_budget = Some(budget);
        self
    }

    /// Sets the token budget.
    ///
    /// # Panics
    ///
    /// Panics if `budget` is zero. A non-positive budget has no meaningful
    /// behaviour — no sample could ever run.
    #[must_use]
    pub fn with_token_budget(mut self, budget: u64) -> Self {
        assert!(budget > 0, "token_budget must be positive, got 0");
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
    pub const fn pacing(mut self, pacing: PacingConfig) -> Self {
        self.pacing = Some(pacing);
        self
    }

    /// Sets the minimum pass rate used for early-termination decisions.
    ///
    /// When set, the execution engine checks after each sample whether
    /// the planned threshold remains reachable
    /// ([`TerminationReason::FailureInevitable`](crate::model::TerminationReason::FailureInevitable))
    /// or is already guaranteed
    /// ([`TerminationReason::SuccessGuaranteed`](crate::model::TerminationReason::SuccessGuaranteed))
    /// and stops early when appropriate.
    ///
    /// Leaving this unset (the default) disables early-termination
    /// entirely — measure, explore, and optimize experiments always run
    /// all planned samples.
    ///
    /// # Panics
    ///
    /// Panics if `rate` is not in [0, 1].
    #[must_use]
    pub fn min_pass_rate(mut self, rate: f64) -> Self {
        assert!(
            (0.0..=1.0).contains(&rate),
            "min_pass_rate must be in [0, 1], got {rate}"
        );
        self.min_pass_rate = Some(rate);
        self
    }

    /// Sets the minimum number of samples the engine must execute before
    /// it is allowed to terminate on `SuccessGuaranteed`.
    ///
    /// Typically sourced from
    /// [`crate::statistics::feasibility::feasibility_check`] so that
    /// early termination never bypasses the sample count required for a
    /// statistically valid verdict. Has no effect on
    /// `FailureInevitable`, which stops as soon as the threshold becomes
    /// unreachable.
    ///
    /// # Panics
    ///
    /// Panics if `floor` is zero.
    #[must_use]
    pub fn min_samples_for_validity(mut self, floor: u32) -> Self {
        assert!(
            floor > 0,
            "min_samples_for_validity must be positive, got 0"
        );
        self.min_samples_for_validity = Some(floor);
        self
    }

    // --- Internal helpers for experiment builders ---

    /// Sets the time budget on this config (consuming and returning it).
    ///
    /// # Panics
    ///
    /// Panics if `budget` is zero.
    #[must_use]
    pub(crate) fn set_time_budget(mut self, budget: Duration) -> Self {
        assert!(
            !budget.is_zero(),
            "time_budget must be positive, got {budget:?}"
        );
        self.time_budget = Some(budget);
        self
    }

    /// Sets the token budget on this config (consuming and returning it).
    ///
    /// # Panics
    ///
    /// Panics if `budget` is zero.
    #[must_use]
    pub(crate) fn set_token_budget(mut self, budget: u64) -> Self {
        assert!(budget > 0, "token_budget must be positive, got 0");
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
    pub const fn pacing_config(&self) -> Option<&PacingConfig> {
        self.pacing.as_ref()
    }

    /// The configured minimum pass rate for early-termination checks,
    /// if any.
    #[must_use]
    pub const fn configured_min_pass_rate(&self) -> Option<f64> {
        self.min_pass_rate
    }

    /// The minimum sample floor that gates `SuccessGuaranteed`
    /// termination, if any.
    #[must_use]
    pub const fn configured_min_samples_for_validity(&self) -> Option<u32> {
        self.min_samples_for_validity
    }
}

/// Pacing constraints for rate-limiting trial execution.
///
/// The three floor-style constraints (`min_ms_per_sample`,
/// `max_requests_per_second`, `max_requests_per_minute`) compose by
/// most-restrictive-wins — each contributes a minimum inter-sample
/// delay, and the folded value is the largest of them. The
/// `max_delay_per_sample` cap is applied *after* the fold, acting as
/// an upper bound on the proactive pacing delay to prevent over-
/// restrictive rate composition from stalling a run.
#[derive(Debug, Clone)]
pub struct PacingConfig {
    min_ms_per_sample: Option<u64>,
    max_requests_per_second: Option<f64>,
    max_requests_per_minute: Option<f64>,
    max_delay_per_sample: Option<u64>,
}

impl PacingConfig {
    /// Creates a pacing config with no constraints.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            min_ms_per_sample: None,
            max_requests_per_second: None,
            max_requests_per_minute: None,
            max_delay_per_sample: None,
        }
    }

    /// Sets the minimum milliseconds between samples.
    ///
    /// # Panics
    ///
    /// Panics if `ms` is zero. A zero minimum is equivalent to no
    /// constraint and almost always indicates a configuration error.
    #[must_use]
    pub fn min_ms_per_sample(mut self, ms: u64) -> Self {
        assert!(ms > 0, "min_ms_per_sample must be positive, got 0");
        self.min_ms_per_sample = Some(ms);
        self
    }

    /// Sets the maximum requests per second.
    ///
    /// # Panics
    ///
    /// Panics if `rps` is not positive.
    #[must_use]
    pub fn max_requests_per_second(mut self, rps: f64) -> Self {
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
    pub fn max_requests_per_minute(mut self, rpm: f64) -> Self {
        assert!(rpm > 0.0, "max requests per minute must be positive");
        self.max_requests_per_minute = Some(rpm);
        self
    }

    /// Configured maximum requests per second, if set.
    #[must_use]
    pub const fn configured_max_requests_per_second(&self) -> Option<f64> {
        self.max_requests_per_second
    }

    /// Configured maximum requests per minute, if set.
    #[must_use]
    pub const fn configured_max_requests_per_minute(&self) -> Option<f64> {
        self.max_requests_per_minute
    }

    /// Sets an upper bound on the proactive pacing delay.
    ///
    /// When set, this caps whatever delay the floor-style constraints
    /// (`min_ms_per_sample`, `max_requests_per_second`,
    /// `max_requests_per_minute`) would otherwise produce. It has no
    /// effect when no floor constraint is set — the effective delay is
    /// zero and no sleep happens.
    ///
    /// # Panics
    ///
    /// Panics if `ms` is zero.
    #[must_use]
    pub fn max_delay_per_sample(mut self, ms: u64) -> Self {
        assert!(ms > 0, "max_delay_per_sample must be positive, got 0");
        self.max_delay_per_sample = Some(ms);
        self
    }

    /// Configured upper bound on the proactive pacing delay, if set.
    #[must_use]
    pub const fn configured_max_delay_per_sample(&self) -> Option<u64> {
        self.max_delay_per_sample
    }

    /// Computes the effective minimum delay between samples in milliseconds.
    ///
    /// Folds the three floor-style constraints via most-restrictive-wins,
    /// then applies the `max_delay_per_sample` cap as an upper bound. If
    /// no floor is configured, the effective delay is zero and the cap
    /// is irrelevant.
    #[must_use]
    pub fn effective_delay_ms(&self) -> u64 {
        let mut floor = 0u64;

        if let Some(ms) = self.min_ms_per_sample {
            floor = floor.max(ms);
        }

        if let Some(rps) = self.max_requests_per_second {
            #[allow(
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation,
                reason = "rate is validated positive; ceiling fits in u64"
            )]
            let ms = (1000.0 / rps).ceil() as u64;
            floor = floor.max(ms);
        }

        if let Some(rpm) = self.max_requests_per_minute {
            #[allow(
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation,
                reason = "rate is validated positive; ceiling fits in u64"
            )]
            let ms = (60_000.0 / rpm).ceil() as u64;
            floor = floor.max(ms);
        }

        match self.max_delay_per_sample {
            Some(cap) if floor > 0 => floor.min(cap),
            _ => floor,
        }
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
        assert!(config.pacing_config().is_none());
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
    #[should_panic(expected = "time_budget must be positive")]
    fn rejects_zero_time_budget() {
        ExecutionConfig::new(100).with_time_budget(Duration::ZERO);
    }

    #[test]
    #[should_panic(expected = "token_budget must be positive")]
    fn rejects_zero_token_budget() {
        ExecutionConfig::new(100).with_token_budget(0);
    }

    #[test]
    fn accepts_min_pass_rate_zero() {
        let config = ExecutionConfig::new(100).min_pass_rate(0.0);
        assert_eq!(config.configured_min_pass_rate(), Some(0.0));
    }

    #[test]
    #[should_panic(expected = "min_pass_rate must be in [0, 1], got -0.5")]
    fn rejects_min_pass_rate_negative() {
        ExecutionConfig::new(100).min_pass_rate(-0.5);
    }

    #[test]
    #[should_panic(expected = "min_pass_rate must be in [0, 1], got 1.1")]
    fn rejects_min_pass_rate_above_one() {
        ExecutionConfig::new(100).min_pass_rate(1.1);
    }

    #[test]
    #[should_panic(expected = "min_pass_rate must be in [0, 1]")]
    fn rejects_min_pass_rate_nan() {
        ExecutionConfig::new(100).min_pass_rate(f64::NAN);
    }

    #[test]
    fn accepts_min_pass_rate_one() {
        let config = ExecutionConfig::new(100).min_pass_rate(1.0);
        assert_eq!(config.configured_min_pass_rate(), Some(1.0));
    }

    #[test]
    #[should_panic(expected = "min_samples_for_validity must be positive")]
    fn rejects_min_samples_for_validity_zero() {
        ExecutionConfig::new(100).min_samples_for_validity(0);
    }

    #[test]
    fn accepts_min_samples_for_validity_one() {
        let config = ExecutionConfig::new(100).min_samples_for_validity(1);
        assert_eq!(config.configured_min_samples_for_validity(), Some(1));
    }

    #[test]
    fn pacing_no_constraints() {
        let pacing = PacingConfig::new();
        assert_eq!(pacing.effective_delay_ms(), 0);
    }

    #[test]
    fn pacing_min_ms_per_sample() {
        let pacing = PacingConfig::new().min_ms_per_sample(200);
        assert_eq!(pacing.effective_delay_ms(), 200);
    }

    #[test]
    fn pacing_rps_constraint() {
        let pacing = PacingConfig::new().max_requests_per_second(5.0);
        assert_eq!(pacing.effective_delay_ms(), 200);
    }

    #[test]
    fn pacing_rpm_constraint() {
        let pacing = PacingConfig::new().max_requests_per_minute(60.0);
        assert_eq!(pacing.effective_delay_ms(), 1000);
    }

    #[test]
    fn pacing_most_restrictive_wins() {
        let pacing = PacingConfig::new()
            .min_ms_per_sample(100)
            .max_requests_per_second(2.0); // 500ms
        assert_eq!(pacing.effective_delay_ms(), 500);
    }

    #[test]
    #[should_panic(expected = "min_ms_per_sample must be positive")]
    fn min_ms_per_sample_rejects_zero() {
        let _ = PacingConfig::new().min_ms_per_sample(0);
    }

    #[test]
    #[should_panic(expected = "max_delay_per_sample must be positive")]
    fn max_delay_per_sample_rejects_zero() {
        let _ = PacingConfig::new().max_delay_per_sample(0);
    }

    #[test]
    #[should_panic(expected = "max requests per second must be positive")]
    fn max_requests_per_second_rejects_zero() {
        let _ = PacingConfig::new().max_requests_per_second(0.0);
    }

    #[test]
    #[should_panic(expected = "max requests per minute must be positive")]
    fn max_requests_per_minute_rejects_zero() {
        let _ = PacingConfig::new().max_requests_per_minute(0.0);
    }

    #[test]
    fn fractional_rps_below_one_yields_interval_longer_than_one_second() {
        // 0.5 rps → one request every 2 seconds = 2000ms.
        let pacing = PacingConfig::new().max_requests_per_second(0.5);
        assert_eq!(pacing.effective_delay_ms(), 2000);
    }

    #[test]
    fn max_delay_cap_overrides_restrictive_floor() {
        // 1 rps → 1000ms floor; 100ms cap wins.
        let pacing = PacingConfig::new()
            .max_requests_per_second(1.0)
            .max_delay_per_sample(100);
        assert_eq!(pacing.effective_delay_ms(), 100);
    }

    #[test]
    fn max_delay_cap_inactive_when_above_floor() {
        // 100ms floor, 500ms cap — floor wins.
        let pacing = PacingConfig::new()
            .min_ms_per_sample(100)
            .max_delay_per_sample(500);
        assert_eq!(pacing.effective_delay_ms(), 100);
    }

    #[test]
    fn max_delay_cap_alone_without_floor_yields_zero() {
        // Cap without any floor constraint: no sleep happens.
        let pacing = PacingConfig::new().max_delay_per_sample(100);
        assert_eq!(pacing.effective_delay_ms(), 0);
    }

    #[test]
    fn composition_takes_most_restrictive_of_the_three_floors() {
        // min_ms=50, rps=5.0 → 200ms, rpm=120.0 → 500ms → effective = 500.
        let pacing = PacingConfig::new()
            .min_ms_per_sample(50)
            .max_requests_per_second(5.0)
            .max_requests_per_minute(120.0);
        assert_eq!(pacing.effective_delay_ms(), 500);
    }
}
