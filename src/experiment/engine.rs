//! The common execution engine for all experiment types and probabilistic tests.

use std::time::{Duration, Instant};

use crate::controls::{ExecutionConfig, TokenRecorder};
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
    /// # Panics
    ///
    /// Panics if `inputs` is empty.
    pub fn run<F>(
        config: &ExecutionConfig,
        inputs: &[String],
        token_recorder: &TokenRecorder,
        mut trial: F,
    ) -> ExecutionResult
    where
        F: FnMut(&str) -> TrialOutcome,
    {
        assert!(!inputs.is_empty(), "inputs must not be empty");

        // Warmup phase: execute and discard
        for i in 0..config.warmup() {
            let input = &inputs[i as usize % inputs.len()];
            let _ = trial(input);
        }

        let start = Instant::now();
        let mut aggregate = SampleAggregate::new();
        let mut termination_reason = TerminationReason::Completed;

        for i in 0..config.samples() {
            // Check time budget
            if let Some(budget) = config.time_budget() {
                if start.elapsed() >= budget {
                    termination_reason = TerminationReason::TimeBudgetExhausted;
                    break;
                }
            }

            // Check token budget
            let tokens_consumed = token_recorder.total()
                + config.static_token_charge().map_or(0, |c| u64::from(i) * c);
            if let Some(budget) = config.token_budget() {
                if tokens_consumed >= budget {
                    termination_reason = TerminationReason::TokenBudgetExhausted;
                    break;
                }
            }

            // Apply pacing delay
            if let Some(pacing) = config.pacing() {
                let delay_ms = pacing.effective_delay_ms();
                if delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                }
            }

            // Execute trial
            let input = &inputs[i as usize % inputs.len()];
            let outcome = trial(input);

            // Record static token charge
            if let Some(charge) = config.static_token_charge() {
                token_recorder.record(charge);
            }

            // Record outcome
            if outcome.is_success() {
                aggregate.record_success(outcome.elapsed());
            } else if let Some(violation) = outcome.violation() {
                aggregate.record_failure(
                    violation,
                    outcome.elapsed(),
                    config.max_example_failures(),
                );
            }

            // Early termination checks
            let remaining = config.samples() - (i + 1);
            if let Some(reason) = check_early_termination(&aggregate, remaining, config) {
                termination_reason = reason;
                break;
            }
        }

        let total_elapsed = start.elapsed();
        let total_tokens = token_recorder.total();
        let samples_executed = aggregate.total();

        let cost = CostSummary::new(total_elapsed, total_tokens, samples_executed);
        let termination = TerminationInfo::new(termination_reason);

        let summary = ExecutionSummary::new(
            config.samples(),
            samples_executed,
            aggregate.successes(),
            aggregate.failures(),
            termination,
            cost,
        );

        ExecutionResult {
            aggregate,
            summary,
            token_recorder: token_recorder.clone(),
        }
    }
}

/// Checks whether early termination is warranted.
///
/// Returns `Some(reason)` if the test can be terminated early,
/// `None` if it should continue.
const fn check_early_termination(
    _aggregate: &SampleAggregate,
    remaining: u32,
    _config: &ExecutionConfig,
) -> Option<TerminationReason> {
    // If there are no remaining samples, no early termination needed
    if remaining == 0 {
        return None;
    }

    // These checks require a threshold to be meaningful.
    // The threshold is not available at engine level — it lives in the
    // probabilistic test layer. Early termination for pass/fail inevitability
    // will be implemented when the probabilistic test calls the engine with
    // a threshold context. For now, the engine only terminates on budget.
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

        let result = ExecutionEngine::run(&config, &inputs, &recorder, always_succeeds);

        assert_eq!(result.summary().samples_executed(), 10);
        assert_eq!(result.summary().successes(), 10);
        assert_eq!(result.summary().failures(), 0);
    }

    #[test]
    fn records_failures() {
        let config = ExecutionConfig::new(5);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, always_fails);

        assert_eq!(result.summary().failures(), 5);
        assert_eq!(result.aggregate().example_failures().len(), 5);
    }

    #[test]
    fn limits_example_failures() {
        let config = ExecutionConfig::new(10).with_max_example_failures(2);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, always_fails);

        assert_eq!(result.summary().failures(), 10);
        assert_eq!(result.aggregate().example_failures().len(), 2);
    }

    #[test]
    fn cycles_inputs_round_robin() {
        let config = ExecutionConfig::new(6);
        let recorder = TokenRecorder::new();
        let inputs = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        let mut seen = Vec::new();
        ExecutionEngine::run(&config, &inputs, &recorder, |input| {
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
        let result = ExecutionEngine::run(&config, &inputs, &recorder, |_input| {
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

        ExecutionEngine::run(&config, &inputs, &recorder, always_succeeds);

        assert_eq!(recorder.total(), 500);
    }

    #[test]
    fn token_budget_terminates_early() {
        let config = ExecutionConfig::new(100)
            .with_static_token_charge(100)
            .with_token_budget(250);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let result = ExecutionEngine::run(&config, &inputs, &recorder, always_succeeds);

        // Should terminate before all 100 samples
        assert!(result.summary().samples_executed() < 100);
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn panics_on_empty_inputs() {
        let config = ExecutionConfig::new(10);
        let recorder = TokenRecorder::new();
        ExecutionEngine::run(&config, &[], &recorder, always_succeeds);
    }
}
