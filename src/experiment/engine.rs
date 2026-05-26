//! The common execution engine for all experiment types and probabilistic tests.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use crate::controls::{ExecutionConfig, RunBudget, TokenRecorder};
use crate::criteria::{CriteriaCounts, CriterionSampleResult};
use crate::model::{
    CostSummary, Defect, ExecutionSummary, SampleAggregate, TerminationInfo, TerminationReason,
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

/// One sample's evaluation: every criterion's per-sample result, plus the
/// wall-clock time the service invocation took (used for latency percentiles
/// over passing samples).
#[derive(Debug, Clone)]
pub struct SampleEvaluation {
    /// The per-criterion results for this sample, in declaration order.
    pub results: Vec<CriterionSampleResult>,
    /// How long the service invocation took.
    pub elapsed: Duration,
}

/// Result of a contract-driven execution run: the per-criterion tallies
/// alongside the composite aggregate and the run summary.
#[derive(Debug)]
pub struct ContractExecutionResult {
    aggregate: SampleAggregate,
    criteria_counts: CriteriaCounts,
    summary: ExecutionSummary,
    token_recorder: TokenRecorder,
}

impl ContractExecutionResult {
    /// The composite aggregate (a sample succeeds iff every criterion passed).
    #[must_use]
    pub const fn aggregate(&self) -> &SampleAggregate {
        &self.aggregate
    }

    /// The per-criterion pass/fail tallies.
    #[must_use]
    pub const fn criteria_counts(&self) -> &CriteriaCounts {
        &self.criteria_counts
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
    /// Runs a contract-driven sampling: the `sample` closure invokes the
    /// service and judges its response, returning one [`SampleEvaluation`] per
    /// call. Warmup, input cycling, budget enforcement, pacing, and early
    /// termination are all applied around that closure.
    ///
    /// Each sample is wrapped in [`catch_unwind`]: a caught panic, like an
    /// explicit `Err(Defect)`, is a defect that **aborts** the run — the engine
    /// stops and returns `Err(Defect)` rather than counting the sample. (A
    /// malformed-but-received response is not a defect; the closure returns it
    /// as a counted criterion failure.)
    ///
    /// A sample counts as a composite success iff **every** criterion passed;
    /// that composite drives the aggregate success tally, the passing-sample
    /// latencies, and early termination, while the per-criterion tallies are
    /// accumulated separately.
    ///
    /// # Errors
    ///
    /// Returns `Err(Defect)` if any sample (warmup or counted) yields a defect.
    ///
    /// # Panics
    ///
    /// Panics if `inputs` is empty.
    pub fn run_contract<I, F>(
        config: &ExecutionConfig,
        inputs: &[I],
        token_recorder: &TokenRecorder,
        run_budget: Option<&RunBudget>,
        mut sample: F,
    ) -> Result<ContractExecutionResult, Defect>
    where
        F: FnMut(&I) -> Result<SampleEvaluation, Defect>,
    {
        assert!(!inputs.is_empty(), "inputs must not be empty");

        for i in 0..config.warmup() {
            let input = &inputs[i as usize % inputs.len()];
            invoke_sample(&mut sample, input)?;
        }

        let start = Instant::now();
        let mut aggregate = SampleAggregate::new();
        let mut criteria_counts = CriteriaCounts::new();
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
            let evaluation = invoke_sample(&mut sample, input)?;

            record_post_trial_consumption(config, token_recorder, run_budget, tokens_before);
            criteria_counts.record_sample(&evaluation.results);
            record_contract_sample(&mut aggregate, &evaluation, config.max_example_failures());

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

        Ok(ContractExecutionResult {
            aggregate,
            criteria_counts,
            summary,
            token_recorder: token_recorder.clone(),
        })
    }
}

/// Invokes the sample closure once, converting a caught panic into a
/// [`Defect`] so a panicking service invocation aborts the run cleanly rather
/// than unwinding through the engine.
fn invoke_sample<I, F>(sample: &mut F, input: &I) -> Result<SampleEvaluation, Defect>
where
    F: FnMut(&I) -> Result<SampleEvaluation, Defect>,
{
    match catch_unwind(AssertUnwindSafe(|| sample(input))) {
        Ok(result) => result,
        Err(panic) => Err(Defect::new(panic_message(panic.as_ref()))),
    }
}

/// Extracts a human-readable message from a caught panic payload.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "service invocation panicked".to_string()
    }
}

/// Records one contract sample into the composite aggregate. The sample is a
/// success iff every criterion passed; otherwise the first failing criterion's
/// violation is recorded as the representative failure.
fn record_contract_sample(
    aggregate: &mut SampleAggregate,
    evaluation: &SampleEvaluation,
    max_example_failures: u32,
) {
    let first_failure = evaluation
        .results
        .iter()
        .find_map(CriterionSampleResult::reason);
    match first_failure {
        None => aggregate.record_success(evaluation.elapsed),
        Some(violation) => {
            aggregate.record_failure(violation, evaluation.elapsed, max_example_failures);
        }
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
///   given the failures already recorded.
/// - `SuccessGuaranteed` — the threshold is already met and will remain
///   met even if every remaining sample fails, subject to the
///   `min_samples_for_validity` floor so early success does not bypass
///   the sample count required for a statistically valid verdict.
///
/// Returns `None` when execution should continue, including when no
/// `min_pass_rate` is configured (measure/explore/optimize callers).
// javai-ref: JVI-BQTS77W — do not remove (resolves in javai-orchestrator)
// javai-ref: JVI-GZFMZXV — do not remove (resolves in javai-orchestrator)
fn check_early_termination(
    aggregate: &SampleAggregate,
    remaining: u32,
    config: &ExecutionConfig,
) -> Option<TerminationReason> {
    let min_pass_rate = config.configured_min_pass_rate()?;
    let required = required_successes(config.samples(), min_pass_rate);
    let successes = aggregate.successes();
    let executed = aggregate.total();

    // Failure-inevitable: can the threshold still be reached?
    if successes + remaining < required {
        return Some(TerminationReason::FailureInevitable);
    }

    // Success-guaranteed: already guaranteed, and enough samples for statistical
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

    // --- contract-driven path (run_contract) ---

    fn eval(results: Vec<CriterionSampleResult>) -> SampleEvaluation {
        SampleEvaluation {
            results,
            elapsed: Duration::from_millis(1),
        }
    }

    #[test]
    fn run_contract_accumulates_per_criterion_and_composite() {
        let config = ExecutionConfig::new(4);
        let recorder = TokenRecorder::new();
        let inputs = vec!["x".to_string()];

        // "a" always passes; "b" fails on even samples. Composite succeeds only
        // when both pass.
        let mut i = 0u32;
        let result =
            ExecutionEngine::run_contract(&config, &inputs, &recorder, None, |_: &String| {
                let b_passes = i % 2 == 1;
                i += 1;
                Ok(eval(vec![
                    CriterionSampleResult::pass("a"),
                    if b_passes {
                        CriterionSampleResult::pass("b")
                    } else {
                        CriterionSampleResult::fail("b", ContractViolation::new("flake", "even"))
                    },
                ]))
            })
            .expect("no defect");

        let counts = result.criteria_counts();
        assert_eq!(counts.get("a").unwrap().pass(), 4);
        assert_eq!(counts.get("b").unwrap().pass(), 2);
        assert_eq!(counts.get("b").unwrap().fail(), 2);
        // Composite success requires every criterion to pass.
        assert_eq!(result.summary().successes(), 2);
        assert_eq!(result.summary().failures(), 2);
    }

    #[test]
    fn run_contract_aborts_on_defect() {
        let config = ExecutionConfig::new(10);
        let recorder = TokenRecorder::new();
        let inputs = vec!["x".to_string()];

        let mut i = 0u32;
        let result =
            ExecutionEngine::run_contract(&config, &inputs, &recorder, None, |_: &String| {
                i += 1;
                if i == 3 {
                    Err(Defect::new("connection refused"))
                } else {
                    Ok(eval(vec![CriterionSampleResult::pass("a")]))
                }
            });

        assert_eq!(result.unwrap_err().message(), "connection refused");
    }

    #[test]
    fn run_contract_converts_panic_to_defect() {
        let config = ExecutionConfig::new(10);
        let recorder = TokenRecorder::new();
        let inputs = vec!["x".to_string()];

        let result =
            ExecutionEngine::run_contract(&config, &inputs, &recorder, None, |_: &String| {
                panic!("kaboom");
            });

        assert_eq!(result.unwrap_err().message(), "kaboom");
    }

    // --- Shared sampling-loop behaviour driven through run_contract ---

    /// A sample that always passes its single criterion.
    fn pass_sample(_: &String) -> Result<SampleEvaluation, Defect> {
        Ok(eval(vec![CriterionSampleResult::pass("a")]))
    }

    /// A sample that always fails its single criterion (composite failure).
    fn fail_sample(_: &String) -> Result<SampleEvaluation, Defect> {
        Ok(eval(vec![CriterionSampleResult::fail(
            "a",
            ContractViolation::new("check", "forced"),
        )]))
    }

    fn summary_of(result: Result<ContractExecutionResult, Defect>) -> ExecutionSummary {
        result.expect("no defect").summary().clone()
    }

    #[test]
    fn runs_all_samples_when_no_budget() {
        let config = ExecutionConfig::new(10);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let summary = summary_of(ExecutionEngine::run_contract(
            &config,
            &inputs,
            &recorder,
            None,
            pass_sample,
        ));

        assert_eq!(summary.samples_executed(), 10);
        assert_eq!(summary.successes(), 10);
    }

    #[test]
    fn cycles_inputs_round_robin() {
        let config = ExecutionConfig::new(6);
        let recorder = TokenRecorder::new();
        let inputs = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        let mut seen = Vec::new();
        let _ =
            ExecutionEngine::run_contract(&config, &inputs, &recorder, None, |input: &String| {
                seen.push(input.clone());
                Ok(eval(vec![CriterionSampleResult::pass("a")]))
            });

        assert_eq!(seen, vec!["a", "b", "c", "a", "b", "c"]);
    }

    #[test]
    fn warmup_invocations_are_discarded() {
        let config = ExecutionConfig::new(3).with_warmup(2);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let mut total_calls = 0u32;
        let summary = summary_of(ExecutionEngine::run_contract(
            &config,
            &inputs,
            &recorder,
            None,
            |_: &String| {
                total_calls += 1;
                Ok(eval(vec![CriterionSampleResult::pass("a")]))
            },
        ));

        assert_eq!(total_calls, 5); // 2 warmup + 3 samples
        assert_eq!(summary.samples_executed(), 3);
    }

    #[test]
    fn records_static_token_charges() {
        let config = ExecutionConfig::new(5).with_static_token_charge(100);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let _ = ExecutionEngine::run_contract(&config, &inputs, &recorder, None, pass_sample);

        assert_eq!(recorder.total(), 500);
    }

    #[test]
    fn failure_inevitable_terminates_after_required_failures() {
        // 100 samples at 0.95 → require 95 successes; best possible after
        // 6 failures is 94/100 → FailureInevitable triggers on sample 6.
        let config = ExecutionConfig::new(100).min_pass_rate(0.95);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let summary = summary_of(ExecutionEngine::run_contract(
            &config,
            &inputs,
            &recorder,
            None,
            fail_sample,
        ));

        assert_eq!(summary.samples_executed(), 6);
        assert_eq!(
            summary.termination().reason(),
            &TerminationReason::FailureInevitable
        );
    }

    #[test]
    fn success_guaranteed_terminates_when_threshold_met_and_floor_cleared() {
        // 100 samples at 0.90, no validity floor → require 90 successes.
        let config = ExecutionConfig::new(100).min_pass_rate(0.90);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let summary = summary_of(ExecutionEngine::run_contract(
            &config,
            &inputs,
            &recorder,
            None,
            pass_sample,
        ));

        assert_eq!(summary.samples_executed(), 90);
        assert_eq!(
            summary.termination().reason(),
            &TerminationReason::SuccessGuaranteed
        );
    }

    #[test]
    fn validity_floor_delays_success_guaranteed() {
        let config = ExecutionConfig::new(100)
            .min_pass_rate(0.90)
            .min_samples_for_validity(95);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let summary = summary_of(ExecutionEngine::run_contract(
            &config,
            &inputs,
            &recorder,
            None,
            pass_sample,
        ));

        assert_eq!(summary.samples_executed(), 95);
        assert_eq!(
            summary.termination().reason(),
            &TerminationReason::SuccessGuaranteed
        );
    }

    #[test]
    fn without_min_pass_rate_runs_to_completion_even_on_all_failures() {
        // Measure / explore / optimize callers never set min_pass_rate;
        // they must always run every planned sample.
        let config = ExecutionConfig::new(50);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let summary = summary_of(ExecutionEngine::run_contract(
            &config,
            &inputs,
            &recorder,
            None,
            fail_sample,
        ));

        assert_eq!(summary.samples_executed(), 50);
        assert_eq!(
            summary.termination().reason(),
            &TerminationReason::Completed
        );
    }

    #[test]
    fn token_budget_terminates_early() {
        let config = ExecutionConfig::new(100)
            .with_static_token_charge(100)
            .with_token_budget(250);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let summary = summary_of(ExecutionEngine::run_contract(
            &config,
            &inputs,
            &recorder,
            None,
            pass_sample,
        ));

        assert!(summary.samples_executed() < 100);
        assert_eq!(
            summary.termination().reason(),
            &TerminationReason::TokenBudgetExhausted
        );
    }

    #[test]
    fn run_scoped_token_budget_terminates_sample_loop() {
        let config = ExecutionConfig::new(100).with_static_token_charge(50);
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];
        let run_budget = RunBudget::new(None, Some(150));

        let summary = summary_of(ExecutionEngine::run_contract(
            &config,
            &inputs,
            &recorder,
            Some(&run_budget),
            pass_sample,
        ));

        assert!(summary.samples_executed() < 100);
        assert_eq!(
            summary.termination().reason(),
            &TerminationReason::RunTokenBudgetExhausted
        );
        assert!(summary.cost().run_scoped().is_some());
    }

    #[test]
    fn pacing_delay_applies_between_samples() {
        use crate::controls::PacingConfig;
        // 4 samples at 50ms pacing = 3 inter-sample delays = ≥150ms total.
        let config = ExecutionConfig::new(4).pacing(PacingConfig::new().min_ms_per_sample(50));
        let recorder = TokenRecorder::new();
        let inputs = vec!["input".to_string()];

        let start = Instant::now();
        let _ = ExecutionEngine::run_contract(&config, &inputs, &recorder, None, pass_sample);
        let elapsed = start.elapsed();

        assert!(
            elapsed >= Duration::from_millis(150),
            "expected ≥ (samples-1)*50ms, got {elapsed:?}"
        );
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn panics_on_empty_inputs() {
        let config = ExecutionConfig::new(10);
        let recorder = TokenRecorder::new();
        let _ = ExecutionEngine::run_contract(&config, &[], &recorder, None, pass_sample);
    }

    // --- Isolated coverage for the private sampling-loop helpers ---

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
