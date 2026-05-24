//! The common execution engine for all experiment types and probabilistic tests.

use std::time::{Duration, Instant};

use crate::controls::{ExecutionConfig, RunBudget, TokenRecorder};
use crate::model::{
    CostSummary, ExecutionSummary, SampleAggregate, TerminationInfo, TerminationReason,
    TrialOutcome,
};

/// Result of an execution run.
#[derive(Debug)]
pub struct ExecutionResult {
    aggregate: SampleAggregate,
    summary: ExecutionSummary,
    token_recorder: TokenRecorder,
}

impl ExecutionResult {
    /// The sample aggregate (successes, failures, distributions).
    #[must_use]
    pub const fn aggregate(&self) -> &SampleAggregate {
        &self.aggregate
    }

    /// The execution summary.
    #[must_use]
    pub const fn summary(&self) -> &ExecutionSummary {
        &self.summary
    }

    /// The token recorder used during execution.
    #[must_use]
    pub const fn token_recorder(&self) -> &TokenRecorder {
        &self.token_recorder
    }
}

/// The common execution engine.
///
/// Drives trial execution with warmup, input cycling, budget enforcement,
/// pacing, and early termination.
pub struct ExecutionEngine;

impl ExecutionEngine {
    /// Runs trials according to the given configuration.
    ///
    /// The `trial` closure is called for each sample with the current input.
    /// It must return a [`TrialOutcome`].
    ///
    /// Inputs are cycled round-robin when sample count exceeds input count.
    ///
    /// When `run_budget` is `Some`, the engine additionally consults the
    /// shared run-scoped budget before every sample and mirrors token
    /// consumption into it after every sample. A depleted run-scoped
    /// budget terminates the sample loop with the run-scoped variant of
    /// [`TerminationReason`], which the verdict dispatch distinguishes
    /// from method-level exhaustion.
    ///
    /// # Panics
    ///
    /// Panics if `inputs` is empty.
    pub fn run<F>(
        config: &ExecutionConfig,
        inputs: &[String],
        token_recorder: &TokenRecorder,
        run_budget: Option<&RunBudget>,
        mut trial: F,
    ) -> ExecutionResult
    where
        F: FnMut(&str) -> TrialOutcome,
    {
        assert!(!inputs.is_empty(), "inputs must not be empty");

        execute_warmup(config, inputs, &mut trial);

        let start = Instant::now();
        let mut aggregate = SampleAggregate::new();
        let mut termination_reason = TerminationReason::Completed;

        for i in 0..config.samples() {
            if let Some(reason) =
                check_pre_sample_budgets(config, token_recorder, run_budget, start)
            {
                termination_reason = reason;
                break;
            }

            apply_pacing(i, config);

            let tokens_before = token_recorder.total();
            let input = &inputs[i as usize % inputs.len()];
            let outcome = trial(input);

            record_post_trial_consumption(config, token_recorder, run_budget, tokens_before);
            record_sample_outcome(&mut aggregate, &outcome, config.max_example_failures());

            let remaining = config.samples() - (i + 1);
            if let Some(reason) = check_early_termination(&aggregate, remaining, config) {
                termination_reason = reason;
                break;
            }
        }

        let cost = build_cost_summary(
            start.elapsed(),
            token_recorder.total(),
            aggregate.total(),
            run_budget,
        );
        let summary = ExecutionSummary::new(
            config.samples(),
            aggregate.total(),
            aggregate.successes(),
            aggregate.failures(),
            TerminationInfo::new(termination_reason),
            cost,
        );

        ExecutionResult {
            aggregate,
            summary,
            token_recorder: token_recorder.clone(),
        }
    }
}

/// Runs the warmup phase, discarding every outcome.
fn execute_warmup<F>(config: &ExecutionConfig, inputs: &[String], trial: &mut F)
where
    F: FnMut(&str) -> TrialOutcome,
{
    for i in 0..config.warmup() {
        let input = &inputs[i as usize % inputs.len()];
        let _ = trial(input);
    }
}

/// Decides whether the upcoming sample must be skipped because a budget
/// is already exhausted.
///
/// Run-scoped budgets are checked before method-level ones: when both
/// scopes exhaust at the same sample, the more general cause is
/// reported. `method_start` is the wall-clock start of the per-method run.
///
/// The static charge for the upcoming sample is not yet in
/// `token_recorder` — it is recorded post-trial — so both the run-scoped
/// and method-level token checks project exactly **one** upcoming charge
/// on top of what is already recorded. Charges for prior samples are
/// already in `token_recorder.total()` and must not be re-projected.
fn check_pre_sample_budgets(
    config: &ExecutionConfig,
    token_recorder: &TokenRecorder,
    run_budget: Option<&RunBudget>,
    method_start: Instant,
) -> Option<TerminationReason> {
    let projected_charge = config.static_token_charge().unwrap_or(0);

    if let Some(rb) = run_budget {
        if rb.time_exhausted() {
            return Some(TerminationReason::RunTimeBudgetExhausted);
        }
        if rb.token_exhausted_at(projected_charge) {
            return Some(TerminationReason::RunTokenBudgetExhausted);
        }
    }

    if let Some(budget) = config.time_budget() {
        if method_start.elapsed() >= budget {
            return Some(TerminationReason::TimeBudgetExhausted);
        }
    }

    if let Some(budget) = config.token_budget() {
        if token_recorder.total() + projected_charge >= budget {
            return Some(TerminationReason::TokenBudgetExhausted);
        }
    }

    None
}

/// Sleeps for the configured pacing delay, if any.
///
/// Skips the delay entirely before the first sample — pacing is
/// about the interval *between* sample starts, so nothing precedes
/// sample zero.
fn apply_pacing(sample_index: u32, config: &ExecutionConfig) {
    if sample_index == 0 {
        return;
    }
    if let Some(pacing) = config.pacing_config() {
        let delay_ms = pacing.effective_delay_ms();
        if delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }
}

/// Records the token consumption attributable to a just-completed
/// trial.
///
/// Adds the configured static charge to the method-level
/// [`TokenRecorder`], then mirrors the resulting total delta (static
/// plus any dynamic charges the trial recorded itself) into the
/// run-scoped budget when one is active.
fn record_post_trial_consumption(
    config: &ExecutionConfig,
    token_recorder: &TokenRecorder,
    run_budget: Option<&RunBudget>,
    tokens_before_trial: u64,
) {
    if let Some(charge) = config.static_token_charge() {
        token_recorder.record(charge);
    }
    if let Some(rb) = run_budget {
        let delta = token_recorder.total().saturating_sub(tokens_before_trial);
        rb.record_tokens(delta);
    }
}

/// Records a trial outcome against the aggregate, honouring the
/// configured cap on captured example failures.
fn record_sample_outcome(
    aggregate: &mut SampleAggregate,
    outcome: &TrialOutcome,
    max_example_failures: u32,
) {
    if outcome.is_success() {
        aggregate.record_success(outcome.elapsed());
    } else if let Some(violation) = outcome.violation() {
        aggregate.record_failure(violation, outcome.elapsed(), max_example_failures);
    }
}

/// Builds the cost summary for a completed run, attaching the
/// run-scoped snapshot when a shared budget participated.
fn build_cost_summary(
    total_elapsed: Duration,
    total_tokens: u64,
    samples_executed: u32,
    run_budget: Option<&RunBudget>,
) -> CostSummary {
    run_budget.map_or_else(
        || CostSummary::new(total_elapsed, total_tokens, samples_executed),
        |rb| {
            CostSummary::new(total_elapsed, total_tokens, samples_executed)
                .with_run_scoped(rb.snapshot())
        },
    )
}

/// Computes the integer number of successes needed to meet a minimum
/// pass rate over the given sample count.
///
/// Uses ceiling so that `required_successes / samples >= min_pass_rate`
/// is the tightest achievable ratio at or above the threshold. A rate of
/// `0.0` yields zero required successes (the trivially-passing case).
///
/// # Panics
///
/// Panics if `min_pass_rate` is outside `[0, 1]` or not finite. These are
/// precondition violations — a caller must never pass an invalid rate.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "target <= samples (u32) since min_pass_rate is in [0, 1]"
)]
fn required_successes(samples: u32, min_pass_rate: f64) -> u32 {
    assert!(
        min_pass_rate.is_finite() && (0.0..=1.0).contains(&min_pass_rate),
        "min_pass_rate must be in [0, 1], got {min_pass_rate}"
    );
    let target = f64::from(samples) * min_pass_rate;
    target.ceil() as u32
}

/// Checks whether early termination is warranted.
///
/// Returns `Some(reason)` when the engine can stop before running all
/// planned samples:
///
/// - `FailureInevitable` — the threshold is mathematically unreachable
///   given the failures already recorded (PT09).
/// - `SuccessGuaranteed` — the threshold is already met and will remain
///   met even if every remaining sample fails (PT10), subject to the
///   `min_samples_for_validity` floor so early success does not bypass
///   the sample count required for a statistically valid verdict.
///
/// Returns `None` when execution should continue, including when no
/// `min_pass_rate` is configured (measure/explore/optimize callers).
fn check_early_termination(
    aggregate: &SampleAggregate,
    remaining: u32,
    config: &ExecutionConfig,
) -> Option<TerminationReason> {
    let min_pass_rate = config.configured_min_pass_rate()?;
    let required = required_successes(config.samples(), min_pass_rate);
    let successes = aggregate.successes();
    let executed = aggregate.total();

    // PT09: can we still reach the threshold?
    if successes + remaining < required {
        return Some(TerminationReason::FailureInevitable);
    }

    // PT10: already guaranteed, and enough samples for statistical
    // validity, and there are still planned samples to skip.
    if remaining > 0 && successes >= required {
        let floor = config.configured_min_samples_for_validity().unwrap_or(0);
        if executed >= floor {
            return Some(TerminationReason::SuccessGuaranteed);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ContractViolation;

    fn always_succeeds(_input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    fn always_fails(_input: &str) -> TrialOutcome {
        TrialOutcome::failure(
            ContractViolation::new("check", "forced"),
            Duration::from_millis(1),
        )
    }

    #[test]
    fn runs_all_samples_when_no_budget() {
        let config = ExecutionConfig::new(10);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);

        assert_eq!(result.summary().samples_executed(), 10);
        assert_eq!(result.summary().successes(), 10);
        assert_eq!(result.summary().failures(), 0);
    }

    #[test]
    fn records_failures() {
        let config = ExecutionConfig::new(5);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_fails);

        assert_eq!(result.summary().failures(), 5);
        assert_eq!(result.aggregate().example_failures().len(), 5);
    }

    #[test]
    fn limits_example_failures() {
        let config = ExecutionConfig::new(10).with_max_example_failures(2);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_fails);

        assert_eq!(result.summary().failures(), 10);
        assert_eq!(result.aggregate().example_failures().len(), 2);
    }

    #[test]
    fn cycles_inputs_round_robin() {
        let config = ExecutionConfig::new(6);
        let recorder = TokenRecorder::new();
        let inputs = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        let mut seen = Vec::new();
        ExecutionEngine::run(&config, &inputs, &recorder, None, |input| {
            seen.push(input.to_string());
            TrialOutcome::success(Duration::ZERO)
        });

        assert_eq!(seen, vec!["a", "b", "c", "a", "b", "c"]);
    }

    #[test]
    fn warmup_invocations_are_discarded() {
        let config = ExecutionConfig::new(3).with_warmup(2);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let mut total_calls = 0u32;
        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, |_input| {
            total_calls += 1;
            TrialOutcome::success(Duration::ZERO)
        });

        assert_eq!(total_calls, 5); // 2 warmup + 3 samples
        assert_eq!(result.summary().samples_executed(), 3);
    }

    #[test]
    fn records_static_token_charges() {
        let config = ExecutionConfig::new(5).with_static_token_charge(100);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);

        assert_eq!(recorder.total(), 500);
    }

    #[test]
    fn token_budget_terminates_early() {
        let config = ExecutionConfig::new(100)
            .with_static_token_charge(100)
            .with_token_budget(250);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);

        // Should terminate before all 100 samples
        assert!(result.summary().samples_executed() < 100);
    }

    #[test]
    fn token_budget_projection_does_not_double_count_prior_charges() {
        // 100 per sample against a 1000 budget. Each pre-sample check
        // projects one upcoming charge on top of what is recorded, so the
        // run affords 9 samples (900 recorded; the 10th would reach 1000
        // and is refused). The earlier defect projected one charge per
        // elapsed sample, terminating at 5 — this pins the fix.
        let config = ExecutionConfig::new(100)
            .with_static_token_charge(100)
            .with_token_budget(1000);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);

        assert_eq!(result.summary().samples_executed(), 9);
        assert_eq!(recorder.total(), 900);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::TokenBudgetExhausted
        );
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn panics_on_empty_inputs() {
        let config = ExecutionConfig::new(10);
        let recorder = TokenRecorder::new();
        ExecutionEngine::run(&config, &[], &recorder, None, always_succeeds);
    }

    // --- PT09: failure-inevitable early termination ---

    #[test]
    fn failure_inevitable_terminates_after_required_failures() {
        // 100 samples at 0.95 → require 95 successes; best possible after
        // 6 failures is 94/100 → FailureInevitable triggers on sample 6.
        let config = ExecutionConfig::new(100).min_pass_rate(0.95);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_fails);

        assert_eq!(result.summary().samples_executed(), 6);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::FailureInevitable
        );
    }

    #[test]
    fn still_reachable_does_not_terminate_early() {
        // 20 samples at 0.90 → require 18 successes. After a stream that
        // allows exactly 18 successes over 20 (2 failures), the run must
        // continue to completion.
        let config = ExecutionConfig::new(20).min_pass_rate(0.90);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let mut call = 0u32;
        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, |_input| {
            call += 1;
            if call <= 2 {
                TrialOutcome::failure(
                    ContractViolation::new("check", "forced"),
                    Duration::from_millis(1),
                )
            } else {
                TrialOutcome::success(Duration::from_millis(1))
            }
        });

        assert_eq!(result.summary().samples_executed(), 20);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::Completed
        );
    }

    // --- PT10: success-guaranteed early termination ---

    #[test]
    fn success_guaranteed_terminates_when_threshold_met_and_floor_cleared() {
        // 100 samples at 0.90, no validity floor → require 90 successes.
        // With all-pass trials this triggers exactly after sample 90.
        let config = ExecutionConfig::new(100).min_pass_rate(0.90);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);

        assert_eq!(result.summary().samples_executed(), 90);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::SuccessGuaranteed
        );
    }

    #[test]
    fn validity_floor_delays_success_guaranteed() {
        // Same threshold, but a floor of 95 forces the engine to keep
        // going past sample 90 until 95 samples have been executed.
        let config = ExecutionConfig::new(100)
            .min_pass_rate(0.90)
            .min_samples_for_validity(95);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);

        assert_eq!(result.summary().samples_executed(), 95);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::SuccessGuaranteed
        );
    }

    #[test]
    fn floor_equal_to_planned_samples_runs_to_completion() {
        // If the floor equals the planned sample count, SuccessGuaranteed
        // can never fire: by the time the floor is cleared, no samples
        // remain. This matches the `remaining > 0` guard.
        let config = ExecutionConfig::new(100)
            .min_pass_rate(0.50)
            .min_samples_for_validity(100);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);

        assert_eq!(result.summary().samples_executed(), 100);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::Completed
        );
    }

    // --- Regression: no min_pass_rate set ---

    #[test]
    fn without_min_pass_rate_engine_runs_to_completion_even_on_all_failures() {
        // Measure / explore / optimize callers never set min_pass_rate;
        // they must always run every planned sample regardless of the
        // success/failure split.
        let config = ExecutionConfig::new(50);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_fails);

        assert_eq!(result.summary().samples_executed(), 50);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::Completed
        );
    }

    #[test]
    fn required_successes_rounds_up() {
        // 10 samples * 0.95 = 9.5 → require 10 successes. A single
        // failure at sample 1 makes the threshold unreachable.
        let config = ExecutionConfig::new(10).min_pass_rate(0.95);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, None, always_fails);

        assert_eq!(result.summary().samples_executed(), 1);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::FailureInevitable
        );
    }

    // --- Run-scoped budget composition ---

    #[test]
    fn run_scoped_time_budget_terminates_sample_loop() {
        let config = ExecutionConfig::new(100);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];
        let run_budget = RunBudget::new(Some(Duration::from_millis(5)), None);

        let result = ExecutionEngine::run(&config, &inputs, &recorder, Some(&run_budget), |_| {
            std::thread::sleep(Duration::from_millis(2));
            TrialOutcome::success(Duration::from_millis(2))
        });

        assert!(result.summary().samples_executed() < 100);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::RunTimeBudgetExhausted
        );
        assert!(result.summary().cost().run_scoped().is_some());
    }

    #[test]
    fn run_scoped_token_budget_terminates_sample_loop() {
        let config = ExecutionConfig::new(100).with_static_token_charge(50);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];
        let run_budget = RunBudget::new(None, Some(150));

        let result = ExecutionEngine::run(
            &config,
            &inputs,
            &recorder,
            Some(&run_budget),
            always_succeeds,
        );

        assert!(result.summary().samples_executed() < 100);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::RunTokenBudgetExhausted
        );
        let snapshot = result.summary().cost().run_scoped().expect("snapshot set");
        assert_eq!(snapshot.token_budget(), Some(150));
        assert!(snapshot.tokens_consumed() > 0);
    }

    #[test]
    fn run_scoped_short_circuits_when_pre_exhausted() {
        let config = ExecutionConfig::new(10);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];
        // Budget of 100 tokens already fully consumed before engine entry.
        let run_budget = RunBudget::new(None, Some(100));
        run_budget.record_tokens(100);

        let result = ExecutionEngine::run(
            &config,
            &inputs,
            &recorder,
            Some(&run_budget),
            always_succeeds,
        );

        assert_eq!(result.summary().samples_executed(), 0);
        assert_eq!(
            result.summary().termination().reason(),
            &TerminationReason::RunTokenBudgetExhausted
        );
    }

    #[test]
    fn run_scoped_tokens_accumulate_across_sequential_runs() {
        let config = ExecutionConfig::new(5).with_static_token_charge(20);
        let run_budget = RunBudget::new(None, Some(500));
        let inputs = vec!["input".to_string()];

        let first_recorder = TokenRecorder::new();
        let first = ExecutionEngine::run(
            &config,
            &inputs,
            &first_recorder,
            Some(&run_budget),
            always_succeeds,
        );
        assert_eq!(first.summary().samples_executed(), 5);
        assert_eq!(run_budget.tokens_consumed(), 100);

        let second_recorder = TokenRecorder::new();
        let second = ExecutionEngine::run(
            &config,
            &inputs,
            &second_recorder,
            Some(&run_budget),
            always_succeeds,
        );
        assert_eq!(second.summary().samples_executed(), 5);
        assert_eq!(run_budget.tokens_consumed(), 200);
    }

    #[test]
    fn run_scoped_and_method_compose_first_exhausted_wins() {
        // A small per-method token budget combined with a generous
        // run-scoped one — the method-level variant fires first.
        let method_tight = ExecutionConfig::new(100)
            .with_static_token_charge(50)
            .with_token_budget(150);
        let generous_run = RunBudget::new(None, Some(100_000));
        let recorder_a = TokenRecorder::new();
        let a = ExecutionEngine::run(
            &method_tight,
            &["input".to_string()],
            &recorder_a,
            Some(&generous_run),
            always_succeeds,
        );
        assert_eq!(
            a.summary().termination().reason(),
            &TerminationReason::TokenBudgetExhausted
        );

        // Invert: generous per-method, tight run-scoped.
        let method_loose = ExecutionConfig::new(100).with_static_token_charge(50);
        let tight_run = RunBudget::new(None, Some(150));
        let recorder_b = TokenRecorder::new();
        let b = ExecutionEngine::run(
            &method_loose,
            &["input".to_string()],
            &recorder_b,
            Some(&tight_run),
            always_succeeds,
        );
        assert_eq!(
            b.summary().termination().reason(),
            &TerminationReason::RunTokenBudgetExhausted
        );
    }

    // --- Pacing (RC08 / RC10) ---

    #[test]
    fn first_sample_runs_without_pacing_delay() {
        use crate::controls::PacingConfig;
        // 500ms pacing floor; a 2-sample run should sleep exactly once
        // (between samples 0 and 1). Total elapsed therefore must be
        // ≥ 500ms (one delay) and comfortably < 1000ms (two delays).
        let config = ExecutionConfig::new(2).pacing(PacingConfig::new().min_ms_per_sample(500));
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let start = Instant::now();
        ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(500),
            "expected ≥ one pacing delay, got {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(900),
            "expected no pre-first-sample delay, got {elapsed:?}"
        );
    }

    #[test]
    fn pacing_delay_applies_between_samples() {
        use crate::controls::PacingConfig;
        // 4 samples at 50ms pacing = 3 inter-sample delays = ≥150ms total.
        let config = ExecutionConfig::new(4).pacing(PacingConfig::new().min_ms_per_sample(50));
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let start = Instant::now();
        ExecutionEngine::run(&config, &inputs, &recorder, None, always_succeeds);
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(150),
            "expected ≥ (samples-1)*50ms, got {elapsed:?}"
        );
    }

    // --- Isolated coverage for the private helpers extracted from `run` ---

    mod check_pre_sample_budgets {
        use super::*;

        #[test]
        fn returns_none_when_no_budgets_configured() {
            let config = ExecutionConfig::new(10);
            let recorder = TokenRecorder::new();
            let start = Instant::now();

            assert!(check_pre_sample_budgets(&config, &recorder, None, start).is_none());
        }

        #[test]
        fn prefers_run_scoped_time_over_method_level() {
            // Both scopes are about to exhaust; the run-scoped cause
            // must win because it is the broader constraint.
            let config = ExecutionConfig::new(10).with_time_budget(Duration::from_millis(1));
            let recorder = TokenRecorder::new();
            let run_budget = RunBudget::new(Some(Duration::from_millis(1)), None);
            std::thread::sleep(Duration::from_millis(5));
            let start = Instant::now() - Duration::from_millis(10);

            let reason = check_pre_sample_budgets(&config, &recorder, Some(&run_budget), start);
            assert_eq!(reason, Some(TerminationReason::RunTimeBudgetExhausted));
        }

        #[test]
        fn prefers_run_scoped_tokens_over_method_level() {
            let config = ExecutionConfig::new(10)
                .with_static_token_charge(50)
                .with_token_budget(50);
            let recorder = TokenRecorder::new();
            let run_budget = RunBudget::new(None, Some(10));

            let reason =
                check_pre_sample_budgets(&config, &recorder, Some(&run_budget), Instant::now());
            assert_eq!(reason, Some(TerminationReason::RunTokenBudgetExhausted));
        }

        #[test]
        fn reports_method_level_time_when_run_scoped_clear() {
            let config = ExecutionConfig::new(10).with_time_budget(Duration::from_millis(1));
            let recorder = TokenRecorder::new();
            let start = Instant::now() - Duration::from_millis(10);

            let reason = check_pre_sample_budgets(&config, &recorder, None, start);
            assert_eq!(reason, Some(TerminationReason::TimeBudgetExhausted));
        }

        #[test]
        fn reports_method_level_tokens_when_consumed_exceeds_budget() {
            let config = ExecutionConfig::new(10).with_token_budget(100);
            let recorder = TokenRecorder::new();
            recorder.record(100);

            let reason = check_pre_sample_budgets(&config, &recorder, None, Instant::now());
            assert_eq!(reason, Some(TerminationReason::TokenBudgetExhausted));
        }

        #[test]
        fn projects_only_the_upcoming_static_charge_not_all_prior() {
            // Three prior samples already recorded (3 × 100). The check
            // for the fourth must project exactly one upcoming charge —
            // 300 + 100 = 400, below the 450 budget — so it proceeds. The
            // earlier defect projected one charge *per elapsed sample*
            // (300 + 4 × 100 = 700), which would have terminated here.
            let config = ExecutionConfig::new(10)
                .with_static_token_charge(100)
                .with_token_budget(450);
            let recorder = TokenRecorder::new();
            recorder.record(300);

            let reason = check_pre_sample_budgets(&config, &recorder, None, Instant::now());
            assert_eq!(reason, None);
        }

        #[test]
        fn terminates_when_recorded_plus_one_upcoming_charge_meets_budget() {
            // 400 recorded + one projected 100 = 500 ≥ 500 budget → stop.
            let config = ExecutionConfig::new(10)
                .with_static_token_charge(100)
                .with_token_budget(500);
            let recorder = TokenRecorder::new();
            recorder.record(400);

            let reason = check_pre_sample_budgets(&config, &recorder, None, Instant::now());
            assert_eq!(reason, Some(TerminationReason::TokenBudgetExhausted));
        }
    }

    mod record_post_trial_consumption {
        use super::*;

        #[test]
        fn records_static_charge_when_configured() {
            let config = ExecutionConfig::new(1).with_static_token_charge(75);
            let recorder = TokenRecorder::new();

            record_post_trial_consumption(&config, &recorder, None, 0);

            assert_eq!(recorder.total(), 75);
        }

        #[test]
        fn no_op_on_recorder_when_no_static_charge() {
            let config = ExecutionConfig::new(1);
            let recorder = TokenRecorder::new();
            recorder.record(20); // dynamic charge already recorded by the trial

            record_post_trial_consumption(&config, &recorder, None, 0);

            assert_eq!(recorder.total(), 20);
        }

        #[test]
        fn mirrors_total_delta_into_run_budget() {
            let config = ExecutionConfig::new(1).with_static_token_charge(30);
            let recorder = TokenRecorder::new();
            // Simulate a trial that recorded 10 dynamic tokens itself.
            recorder.record(10);
            let tokens_before = 0; // snapshot captured before the trial ran
            let run_budget = RunBudget::new(None, Some(1_000));

            record_post_trial_consumption(&config, &recorder, Some(&run_budget), tokens_before);

            // 10 dynamic + 30 static = 40 mirrored into the run-scoped budget.
            assert_eq!(run_budget.tokens_consumed(), 40);
        }

        #[test]
        fn saturating_delta_protects_against_non_monotonic_recorder() {
            // Defensive: tokens_before_trial greater than current total
            // (a clone/reset path) must not panic or underflow.
            let config = ExecutionConfig::new(1);
            let recorder = TokenRecorder::new();
            let run_budget = RunBudget::new(None, Some(1_000));

            record_post_trial_consumption(&config, &recorder, Some(&run_budget), 500);

            assert_eq!(run_budget.tokens_consumed(), 0);
        }
    }

    mod record_sample_outcome {
        use super::*;

        #[test]
        fn success_increments_successes() {
            let mut aggregate = SampleAggregate::new();
            let outcome = TrialOutcome::success(Duration::from_millis(3));

            record_sample_outcome(&mut aggregate, &outcome, 5);

            assert_eq!(aggregate.successes(), 1);
            assert_eq!(aggregate.failures(), 0);
        }

        #[test]
        fn failure_records_and_caps_example_list() {
            let mut aggregate = SampleAggregate::new();
            let violation = ContractViolation::new("check", "forced");
            let outcome = TrialOutcome::failure(violation, Duration::from_millis(1));

            record_sample_outcome(&mut aggregate, &outcome, 2);
            record_sample_outcome(&mut aggregate, &outcome, 2);
            record_sample_outcome(&mut aggregate, &outcome, 2);

            assert_eq!(aggregate.failures(), 3);
            assert_eq!(aggregate.example_failures().len(), 2);
        }
    }

    mod build_cost_summary {
        use super::*;

        #[test]
        fn omits_run_scoped_snapshot_without_budget() {
            let cost = build_cost_summary(Duration::from_millis(100), 500, 10, None);

            assert!(cost.run_scoped().is_none());
        }

        #[test]
        fn attaches_snapshot_when_run_budget_present() {
            let run_budget = RunBudget::new(None, Some(1_000));
            run_budget.record_tokens(250);

            let cost = build_cost_summary(Duration::from_millis(100), 500, 10, Some(&run_budget));

            let snapshot = cost.run_scoped().expect("snapshot expected");
            assert_eq!(snapshot.tokens_consumed(), 250);
            assert_eq!(snapshot.token_budget(), Some(1_000));
        }
    }
}
