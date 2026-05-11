//! Probabilistic test execution and verdict production.

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::ExecutionEngine;
use crate::latency::{
    LatencyDimension, LatencyEnforcementMode, LatencyThresholds, enforcement, resolver,
};
use crate::model::{
    BudgetExhaustedBehavior, ExecutionSummary, ExpirationInfo, PacingSummary, TerminationReason,
    TestIdentity, TestIntent, ThresholdOrigin, TrialOutcome, Warning,
};
use crate::ptest::approach;
use crate::ptest::builder::ThresholdApproach;
use crate::ptest::diagnostics;
use crate::spec::{BaselineSpec, SpecResolver};
use crate::statistics::types::{DerivedThreshold, FeasibilityResult};
use crate::statistics::{evaluator, feasibility, proportion};
use crate::usecase::CovariateContext;
use crate::verdict::{
    BaselineProvenance, FunctionalDimension, SpecProvenance, StatisticalAnalysis, Verdict,
    VerdictRecord,
};

/// What constitutes acceptable service behaviour.
///
/// Groups the functional success-rate criteria and the latency criteria
/// as peers — both are dimensions of the same question: "is this service
/// good enough?" Provenance fields describe where the criteria came from.
#[derive(Debug, Clone)]
pub struct AssessmentCriteria {
    /// How to derive the success-rate threshold.
    pub approach: ThresholdApproach,
    /// Whether this is a verification or smoke test.
    pub intent: TestIntent,
    /// Where the threshold originates (empirical, SLA, etc.).
    pub threshold_origin: ThresholdOrigin,
    /// Human-readable contract reference, if any.
    pub contract_ref: Option<String>,
    /// Latency acceptance criteria.
    pub latency: LatencyConfig,
    /// If true, an expired baseline forces the verdict to `Fail` rather
    /// than only emitting a warning.
    pub fail_on_expired_baseline: bool,
    /// What to do when a budget is exhausted. Applied to the runner's
    /// synthesized execution config when the caller did not supply an
    /// explicit `ExecutionConfig`. An explicit config carries its own
    /// setting and is respected as-is.
    pub on_budget_exhausted: Option<BudgetExhaustedBehavior>,
}

/// How to find and interpret empirical reference data.
///
/// The resolved baseline feeds both the functional assessment (observed
/// success rate for threshold derivation) and the latency assessment
/// (observed latencies for derived percentile thresholds).
#[derive(Debug, Clone, Default)]
pub struct BaselineContext {
    /// Filesystem resolver for baseline specs.
    pub spec_resolver: Option<SpecResolver>,
    /// A pre-loaded baseline spec, bypassing the resolver.
    pub pre_resolved_spec: Option<crate::spec::BaselineSpec>,
    /// Covariate context for covariate-aware baseline selection.
    pub covariate_context: Option<CovariateContext>,
}

/// Latency configuration carried into the runner.
#[derive(Debug, Clone, Copy, Default)]
pub struct LatencyConfig {
    /// Explicit thresholds declared on the builder.
    pub thresholds: LatencyThresholds,
    /// Explicit enforcement mode from the builder, if any.
    pub baseline_mode: Option<LatencyEnforcementMode>,
    /// Confidence used when deriving baseline thresholds.
    pub baseline_confidence: f64,
}

/// The result of a probabilistic test.
///
/// Wraps a [`VerdictRecord`] containing the verdict, statistical analysis,
/// and all supporting evidence.
///
/// # Examples
///
/// ```
/// use feotest::ptest::ProbabilisticTestBuilder;
/// use feotest::ptest::builder::ThresholdApproach;
/// use feotest::model::TrialOutcome;
/// use feotest::verdict::Verdict;
/// use std::time::Duration;
///
/// let inputs = vec!["input".to_string()];
/// let result = ProbabilisticTestBuilder::new("my-service", &inputs,
///     |_| TrialOutcome::success(Duration::from_millis(1)),
/// )
/// .approach(ThresholdApproach::ThresholdFirst {
///     samples: 30,
///     min_pass_rate: 0.80,
/// })
/// .run();
///
/// let record = result.verdict_record();
/// assert_eq!(record.verdict(), Verdict::Pass);
/// assert!(record.statistical_analysis().is_some());
/// assert!(record.functional().pass_rate() > 0.80);
/// ```
#[derive(Debug)]
pub struct ProbabilisticTestResult {
    verdict_record: VerdictRecord,
    approach: ThresholdApproach,
}

impl ProbabilisticTestResult {
    /// The full verdict record.
    #[must_use]
    pub const fn verdict_record(&self) -> &VerdictRecord {
        &self.verdict_record
    }

    /// The threshold approach used for this test.
    #[must_use]
    pub const fn approach(&self) -> &ThresholdApproach {
        &self.approach
    }

    /// Whether the test passed across all dimensions.
    ///
    /// Combines the functional verdict with the latency dimension when
    /// present. Advisory latency violations do not affect this result.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.verdict_record.passed()
    }
}

/// Executes a probabilistic test and produces a verdict.
pub fn execute<F>(
    use_case_id: &str,
    inputs: &[String],
    trial: F,
    criteria: &AssessmentCriteria,
    baseline: BaselineContext,
    config_overrides: Option<&ExecutionConfig>,
) -> ProbabilisticTestResult
where
    F: FnMut(&str) -> TrialOutcome,
{
    let mut warnings: Vec<Warning> = Vec::new();

    let baseline_spec = resolve_baseline(baseline, use_case_id, &mut warnings);

    let (samples, derived_threshold) = approach::resolve_threshold(
        &criteria.approach,
        baseline_spec.as_ref().map(|s| &s.statistics),
        baseline_spec.as_ref().map(|s| &s.execution),
    );

    let resolved_confidence = approach::resolved_confidence(&criteria.approach);
    let feas =
        feasibility::feasibility_check(samples, derived_threshold.value(), resolved_confidence);

    enforce_feasibility(use_case_id, criteria.intent, &feas, &mut warnings);

    let config = synthesise_execution_config(
        config_overrides,
        criteria,
        samples,
        &derived_threshold,
        &feas,
    );

    let token_recorder = TokenRecorder::new();
    let exec_result = ExecutionEngine::run(
        &config,
        inputs,
        &token_recorder,
        crate::controls::run::current(),
        trial,
    );

    let summary = exec_result.summary();
    let aggregate = exec_result.aggregate();

    let mut verdict = compute_stats_verdict(summary, &derived_threshold);
    verdict = apply_budget_exhaustion_policy(summary, &config, verdict, &mut warnings);

    let expiration_info = baseline_spec
        .as_ref()
        .map(crate::spec::expiration::evaluate);
    verdict = apply_expiration_policy(
        expiration_info.as_ref(),
        criteria.fail_on_expired_baseline,
        verdict,
        &mut warnings,
    );

    record_smoke_normative_warning(criteria, &mut warnings);

    let analysis = build_analysis(summary, &derived_threshold, criteria.threshold_origin);
    let provenance = build_provenance(
        criteria.threshold_origin,
        baseline_spec.as_ref(),
        criteria.contract_ref.as_deref(),
        expiration_info,
    );
    let functional = FunctionalDimension::new(
        summary.successes(),
        summary.failures(),
        aggregate.failure_distribution().to_vec(),
    );

    let latency_dimension = build_latency_dimension(
        &criteria.latency,
        aggregate.successful_latencies(),
        baseline_spec.as_ref(),
        &mut warnings,
    );

    let baseline_prov = baseline_spec.as_ref().map(build_baseline_provenance);

    let identity = TestIdentity::new(use_case_id);
    let mut builder = VerdictRecord::builder(
        identity,
        verdict,
        criteria.intent,
        summary.clone(),
        functional,
    )
    .statistical_analysis(analysis)
    .spec_provenance(provenance);
    if let Some(bp) = baseline_prov {
        builder = builder.baseline_provenance(bp);
    }
    if let Some(pacing) = config.pacing_config() {
        builder = builder.pacing(PacingSummary::from_config(pacing));
    }
    if let Some(dim) = latency_dimension {
        builder = builder.latency(dim);
    }
    for w in warnings {
        builder = builder.warning(w);
    }

    ProbabilisticTestResult {
        verdict_record: builder.build(),
        approach: criteria.approach.clone(),
    }
}

/// Resolves the baseline spec via the context's pre-resolved slot or
/// its resolver, whichever is set. Warnings from the resolution path
/// are pushed into the caller's vec.
fn resolve_baseline(
    baseline: BaselineContext,
    use_case_id: &str,
    warnings: &mut Vec<Warning>,
) -> Option<BaselineSpec> {
    baseline.pre_resolved_spec.or_else(|| {
        baseline.spec_resolver.as_ref().and_then(|resolver| {
            crate::ptest::baseline::resolve(
                resolver,
                use_case_id,
                baseline.covariate_context.as_ref(),
                warnings,
            )
        })
    })
}

/// Panics (under `Verification` intent) or warns (under `Smoke` intent)
/// when the configuration is statistically infeasible.
fn enforce_feasibility(
    use_case_id: &str,
    intent: TestIntent,
    feas: &FeasibilityResult,
    warnings: &mut Vec<Warning>,
) {
    if feas.feasible() {
        return;
    }
    match intent {
        TestIntent::Verification => {
            panic!(
                "\n\n{}\n",
                diagnostics::infeasibility_message(use_case_id, feas, false),
            );
        }
        TestIntent::Smoke => {
            warnings.push(Warning::new(
                "UNDERSIZED",
                diagnostics::infeasibility_message(use_case_id, feas, false),
            ));
        }
    }
}

/// Produces the execution config the engine will run against. Honours
/// an explicit `config_overrides` as final; otherwise synthesises a
/// default config, folding the criteria's budget-exhaustion preference
/// in before the final `min_pass_rate` / `min_samples_for_validity`
/// adjustments.
fn synthesise_execution_config(
    config_overrides: Option<&ExecutionConfig>,
    criteria: &AssessmentCriteria,
    samples: u32,
    derived_threshold: &DerivedThreshold,
    feas: &FeasibilityResult,
) -> ExecutionConfig {
    config_overrides
        .cloned()
        .unwrap_or_else(|| {
            let mut c = ExecutionConfig::new(samples);
            if let Some(behaviour) = criteria.on_budget_exhausted {
                c = c.with_on_budget_exhausted(behaviour);
            }
            c
        })
        .min_pass_rate(derived_threshold.value())
        .min_samples_for_validity(feas.minimum_samples())
}

/// Runs the statistical evaluator against the completed sample set and
/// projects its pass/fail onto a `Verdict`. An empty sample set is
/// reported as `Verdict::Fail` without invoking the evaluator (which
/// requires a non-empty sample set).
fn compute_stats_verdict(
    summary: &ExecutionSummary,
    derived_threshold: &DerivedThreshold,
) -> Verdict {
    if summary.samples_executed() == 0 {
        return Verdict::Fail;
    }
    if evaluator::evaluate(
        summary.successes(),
        summary.samples_executed(),
        derived_threshold,
    )
    .passed()
    {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

/// Adjusts the stats-derived verdict in response to a budget-exhausted
/// termination. Zero completed samples always force `Verdict::Fail`.
/// Otherwise the configured `BudgetExhaustedBehavior` decides: `Fail`
/// forces `Verdict::Fail` with a `BUDGET_EXHAUSTED` warning;
/// `EvaluatePartial` preserves the stats-derived verdict with a
/// `BUDGET_EXHAUSTED_PARTIAL` warning. Non-budget terminations pass
/// the verdict through unchanged.
fn apply_budget_exhaustion_policy(
    summary: &ExecutionSummary,
    config: &ExecutionConfig,
    verdict: Verdict,
    warnings: &mut Vec<Warning>,
) -> Verdict {
    let budget_name = match summary.termination().reason() {
        TerminationReason::TimeBudgetExhausted => "time",
        TerminationReason::TokenBudgetExhausted => "token",
        TerminationReason::RunTimeBudgetExhausted => "run-scoped time",
        TerminationReason::RunTokenBudgetExhausted => "run-scoped token",
        _ => return verdict,
    };
    let executed = summary.samples_executed();
    let planned = summary.samples_planned();
    let consumption = consumption_phrase(summary.termination().reason(), summary, config);

    if executed == 0 {
        warnings.push(Warning::new(
            "BUDGET_EXHAUSTED_NO_SAMPLES",
            format!(
                "{budget_name} budget exhausted before any sample completed \
                 ({consumption})"
            ),
        ));
        return Verdict::Fail;
    }

    match config.on_budget_exhausted() {
        BudgetExhaustedBehavior::Fail => {
            warnings.push(Warning::new(
                "BUDGET_EXHAUSTED",
                format!(
                    "{budget_name} budget exhausted ({consumption}); \
                     completed {executed}/{planned} samples; failing per \
                     budget exhaustion policy"
                ),
            ));
            Verdict::Fail
        }
        BudgetExhaustedBehavior::EvaluatePartial => {
            warnings.push(Warning::new(
                "BUDGET_EXHAUSTED_PARTIAL",
                format!(
                    "{budget_name} budget exhausted ({consumption}); \
                     completed {executed}/{planned} samples; evaluating \
                     partial results"
                ),
            ));
            verdict
        }
    }
}

/// Given the baseline's expiration info, adjusts the verdict and emits
/// a warning when expired. When `fail_on_expired` is set, an expired
/// baseline forces `Verdict::Fail`; otherwise the verdict passes
/// through with an informational warning. Non-expired (or absent)
/// expiration info is a no-op.
fn apply_expiration_policy(
    expiration_info: Option<&ExpirationInfo>,
    fail_on_expired: bool,
    verdict: Verdict,
    warnings: &mut Vec<Warning>,
) -> Verdict {
    let expired = expiration_info
        .is_some_and(|info| matches!(info.status(), crate::model::ExpirationStatus::Expired));

    if !expired {
        return verdict;
    }
    if fail_on_expired {
        warnings.push(Warning::new(
            "BASELINE_EXPIRED",
            "baseline has expired; failing per fail_on_expired_baseline",
        ));
        Verdict::Fail
    } else {
        warnings.push(Warning::new(
            "BASELINE_EXPIRED",
            "baseline has expired; re-run the measure experiment to refresh it",
        ));
        verdict
    }
}

/// Emits a `SMOKE_NORMATIVE` warning when a smoke test runs against a
/// normative threshold. The verdict is informational in that combination
/// — a normative contract cannot be empirically verified from a smoke
/// sample size.
fn record_smoke_normative_warning(criteria: &AssessmentCriteria, warnings: &mut Vec<Warning>) {
    if criteria.intent == TestIntent::Smoke && criteria.threshold_origin.is_normative() {
        warnings.push(Warning::new(
            "SMOKE_NORMATIVE",
            "Smoke test against normative threshold — verdict is not evidential",
        ));
    }
}

/// Derives a baseline provenance record from the resolved spec —
/// filename, timestamp, sample count, observed rate, and declared
/// minimum rate.
fn build_baseline_provenance(spec: &BaselineSpec) -> BaselineProvenance {
    BaselineProvenance::new(
        format!("{}.yaml", spec.use_case_id),
        spec.generated_at.clone(),
        spec.execution.samples_executed,
        spec.statistics.success_rate.observed,
        spec.requirements.min_pass_rate,
    )
}

/// Renders a "consumed X of Y" phrase for the exhausted budget. Method-
/// level variants draw actuals from the cost summary and the configured
/// ceiling from the execution config; run-scoped variants draw both
/// consumption and cap from the run-scoped snapshot stamped onto the
/// cost summary at termination time. Returns an empty string for
/// non-budget termination reasons — this helper is only called in the
/// budget-exhausted branch.
fn consumption_phrase(
    reason: &TerminationReason,
    summary: &crate::model::ExecutionSummary,
    config: &ExecutionConfig,
) -> String {
    match reason {
        TerminationReason::TimeBudgetExhausted => {
            let consumed = summary.cost().total_time();
            let budget = config.time_budget().unwrap_or_default();
            format!("consumed {consumed:?} of {budget:?}")
        }
        TerminationReason::TokenBudgetExhausted => {
            let consumed = summary.cost().total_tokens();
            let budget = config.token_budget().unwrap_or(0);
            format!("consumed {consumed} of {budget} tokens")
        }
        TerminationReason::RunTimeBudgetExhausted => {
            let snapshot = summary
                .cost()
                .run_scoped()
                .expect("run-scoped termination implies snapshot presence");
            let consumed = snapshot.time_consumed();
            let budget = snapshot.time_budget().unwrap_or_default();
            format!("consumed {consumed:?} of {budget:?}")
        }
        TerminationReason::RunTokenBudgetExhausted => {
            let snapshot = summary
                .cost()
                .run_scoped()
                .expect("run-scoped termination implies snapshot presence");
            let consumed = snapshot.tokens_consumed();
            let budget = snapshot.token_budget().unwrap_or(0);
            format!("consumed {consumed} of {budget} tokens")
        }
        _ => String::new(),
    }
}

/// Resolves thresholds, computes percentiles, and builds the latency
/// dimension. Returns `None` when no latency assertions apply.
fn build_latency_dimension(
    config: &LatencyConfig,
    successful_latencies: &[std::time::Duration],
    baseline_spec: Option<&crate::spec::BaselineSpec>,
    warnings: &mut Vec<Warning>,
) -> Option<LatencyDimension> {
    let baseline_latency = baseline_spec.and_then(|s| s.statistics.latency_distribution.as_ref());
    if config.thresholds.is_empty() && baseline_latency.is_none() {
        return None;
    }

    let mode = enforcement::resolved_mode_from_env(config.baseline_mode);
    let resolved = resolver::resolve(
        &config.thresholds,
        baseline_latency,
        config.baseline_confidence,
        mode,
    );
    if resolved.is_empty() {
        return None;
    }

    for t in &resolved {
        if !t.feasible() {
            warnings.push(Warning::new(
                "LATENCY_INFEASIBLE",
                format!(
                    "{} not evaluated: baseline has too few successful samples",
                    t.percentile()
                ),
            ));
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "millisecond latencies fit in f64 mantissa"
    )]
    let latencies_f64: Vec<f64> = successful_latencies
        .iter()
        .map(|d| d.as_millis() as f64)
        .collect();
    Some(LatencyDimension::build(&latencies_f64, &resolved))
}

/// Builds the statistical analysis component of a verdict.
fn build_analysis(
    summary: &crate::model::ExecutionSummary,
    derived_threshold: &crate::statistics::types::DerivedThreshold,
    threshold_origin: ThresholdOrigin,
) -> StatisticalAnalysis {
    let confidence_level = derived_threshold.context().confidence().value();

    let (se, wilson_lower) = if summary.samples_executed() > 0 {
        let se = proportion::standard_error(summary.successes(), summary.samples_executed());
        let lower = proportion::lower_bound(
            summary.successes(),
            summary.samples_executed(),
            derived_threshold.context().confidence(),
        );
        (se, lower)
    } else {
        (0.0, 0.0)
    };

    let mut analysis = StatisticalAnalysis::new(
        confidence_level,
        se,
        wilson_lower,
        derived_threshold.value(),
        threshold_origin,
    );

    if summary.samples_executed() > 0 {
        let z = proportion::z_test_statistic(
            summary.observed_pass_rate(),
            derived_threshold.value(),
            summary.samples_executed(),
        );
        let p = proportion::one_sided_p_value(z);
        analysis = analysis.with_test_results(z, p);
    }

    analysis
}

/// Builds spec provenance from the baseline spec and contract ref.
fn build_provenance(
    threshold_origin: ThresholdOrigin,
    baseline_spec: Option<&crate::spec::BaselineSpec>,
    contract_ref: Option<&str>,
    expiration_info: Option<crate::model::ExpirationInfo>,
) -> SpecProvenance {
    let mut provenance = SpecProvenance::new(threshold_origin);
    if let Some(spec) = baseline_spec {
        provenance = provenance.with_spec_filename(format!("{}.yaml", spec.use_case_id));
        if let Some(info) = expiration_info
            && !matches!(info.status(), crate::model::ExpirationStatus::NoExpiration)
        {
            provenance = provenance.with_expiration(info);
        }
    }
    if let Some(cref) = contract_ref {
        provenance = provenance.with_contract_ref(cref);
    }
    provenance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ptest::ProbabilisticTestBuilder;
    use std::time::Duration;

    fn always_succeeds(_input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    fn mostly_succeeds(input: &str) -> TrialOutcome {
        // Deterministic "failure" for specific inputs
        if input == "fail" {
            TrialOutcome::failure(
                crate::model::ContractViolation::new("check", "forced"),
                Duration::from_millis(1),
            )
        } else {
            TrialOutcome::success(Duration::from_millis(1))
        }
    }

    #[test]
    fn threshold_first_all_pass() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test-uc", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.90,
            })
            .run();

        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
        assert_eq!(result.verdict_record().intent(), TestIntent::Verification);
    }

    #[test]
    fn threshold_first_below_threshold() {
        // 8 out of 10 inputs are "ok", 2 are "fail" — cycling 100 samples gives 80% pass rate.
        // Threshold 0.90 is feasible at 100 samples; observed 80% fails the test.
        let inputs: Vec<String> = (0..10)
            .map(|i| {
                if i < 2 {
                    "fail".to_string()
                } else {
                    "ok".to_string()
                }
            })
            .collect();

        let result = ProbabilisticTestBuilder::new("test-uc", &inputs, mostly_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.90,
            })
            .run();

        assert_eq!(result.verdict_record().verdict(), Verdict::Fail);
    }

    #[test]
    fn verdict_record_has_statistical_analysis() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test-uc", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 30,
                min_pass_rate: 0.80,
            })
            .threshold_origin(ThresholdOrigin::Empirical)
            .run();

        let record = result.verdict_record();
        assert!(record.statistical_analysis().is_some());
        let stats = record.statistical_analysis().unwrap();
        assert!((stats.threshold() - 0.80).abs() < 1e-10);
        assert!(stats.p_value().is_some());
        assert!(stats.test_statistic().is_some());
    }

    #[test]
    fn smoke_intent_is_recorded() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test-uc", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 10,
                min_pass_rate: 0.80,
            })
            .intent(TestIntent::Smoke)
            .run();

        assert_eq!(result.verdict_record().intent(), TestIntent::Smoke);
    }

    #[test]
    fn spec_provenance_includes_threshold_origin() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test-uc", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 30,
                min_pass_rate: 0.90,
            })
            .threshold_origin(ThresholdOrigin::Sla)
            .contract_ref("API SLA v3.2 §2.1")
            .run();

        let prov = result.verdict_record().spec_provenance().unwrap();
        assert_eq!(prov.threshold_origin(), ThresholdOrigin::Sla);
        assert_eq!(prov.contract_ref(), Some("API SLA v3.2 §2.1"));
    }

    #[test]
    fn sample_size_first_with_spec() {
        // Write a spec, then run a test against it
        let dir = tempfile::tempdir().unwrap();
        let resolver = crate::spec::SpecResolver::with_dir(dir.path());

        // Create a baseline via measure experiment
        struct SpecTestUc;
        impl crate::usecase::UseCase for SpecTestUc {
            fn id(&self) -> &str {
                "spec-test"
            }
        }
        let inputs = vec!["input".to_string()];
        let measure_result = crate::experiment::MeasureExperiment::builder()
            .use_case_id("spec-test")
            .use_case(|| ())
            .samples(200)
            .inputs(&inputs)
            .trial(|(): &(), input| always_succeeds(input))
            .baseline_dir(dir.path())
            .build()
            .run();

        assert!(measure_result.spec_path().is_some());

        // Now run a probabilistic test using the spec
        let result = ProbabilisticTestBuilder::new("spec-test", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 200,
                confidence: 0.95,
            })
            .spec_resolver(resolver)
            .threshold_origin(ThresholdOrigin::Empirical)
            .run();

        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
        let stats = result.verdict_record().statistical_analysis().unwrap();
        assert!(stats.threshold() > 0.0);
    }

    #[test]
    fn confidence_first_with_spec() {
        let dir = tempfile::tempdir().unwrap();

        struct ConfTestUc;
        impl crate::usecase::UseCase for ConfTestUc {
            fn id(&self) -> &str {
                "conf-test"
            }
        }
        let inputs = vec!["input".to_string()];
        crate::experiment::MeasureExperiment::builder()
            .use_case_id("conf-test")
            .use_case(|| ())
            .samples(200)
            .inputs(&inputs)
            .trial(|(): &(), input| always_succeeds(input))
            .baseline_dir(dir.path())
            .build()
            .run();

        let resolver = crate::spec::SpecResolver::with_dir(dir.path());
        let result = ProbabilisticTestBuilder::new("conf-test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 0.95,
                min_detectable_effect: 0.003,
                power: 0.80,
            })
            .spec_resolver(resolver)
            .run();

        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
        // Confidence-first should compute samples > 0
        assert!(result.verdict_record().execution().samples_executed() > 0);
    }

    #[test]
    #[should_panic(expected = "UNDER-SPECIFIED")]
    fn panics_without_approach() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test-uc", &inputs, always_succeeds).run();
    }

    #[test]
    #[should_panic(expected = "integrity check failed")]
    fn threshold_first_with_covariates_panics_on_tampered_baseline() {
        use crate::spec::namer::CovariateProfile;
        use crate::usecase::{CovariateCategory, CovariateDeclaration, UseCase};

        // Write a valid baseline with covariates
        let dir = tempfile::tempdir().unwrap();

        struct CovUc;
        impl UseCase for CovUc {
            fn id(&self) -> &str {
                "cov-integrity"
            }
            fn covariates(&self) -> Vec<CovariateDeclaration> {
                vec![CovariateDeclaration::new(
                    "model",
                    CovariateCategory::ExternalDependency,
                )]
            }
            fn resolve_covariates(&self) -> CovariateProfile {
                CovariateProfile::builder().put("model", "gpt-4o").build()
            }
        }

        let uc = CovUc;
        let inputs = vec!["input".to_string()];
        let profile = CovariateProfile::builder().put("model", "gpt-4o").build();

        crate::experiment::MeasureExperiment::builder()
            .use_case_id("cov-integrity")
            .use_case(|| ())
            .samples(100)
            .inputs(&inputs)
            .trial(|(): &(), input| always_succeeds(input))
            .baseline_dir(dir.path())
            .covariates(vec!["model".to_string()], profile)
            .build()
            .run();

        // Tamper with the written baseline
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml") {
                let content = std::fs::read_to_string(&path).unwrap();
                let tampered = content.replace("minPassRate: ", "minPassRate: 0.1\n# was: ");
                std::fs::write(&path, tampered).unwrap();
            }
        }

        // Threshold-first with covariates: the resolver must still be
        // constructed, the baseline must still be loaded and verified,
        // and the integrity failure must panic — not silently succeed.
        let resolver = crate::spec::SpecResolver::with_dir(dir.path());
        ProbabilisticTestBuilder::new("cov-integrity", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.80,
            })
            .spec_resolver(resolver)
            .threshold_origin(ThresholdOrigin::Sla)
            .covariate_source(&uc)
            .run();
    }

    #[test]
    #[should_panic(expected = "integrity check failed")]
    fn resolve_panics_on_tampered_baseline_without_covariates() {
        // Write a valid baseline, tamper with it, then resolve via the
        // non-covariate path. The integrity error must still panic.
        let dir = tempfile::tempdir().unwrap();

        struct SimpleUc;
        impl crate::usecase::UseCase for SimpleUc {
            fn id(&self) -> &str {
                "integrity-simple"
            }
        }

        let inputs = vec!["input".to_string()];
        crate::experiment::MeasureExperiment::builder()
            .use_case_id("integrity-simple")
            .use_case(|| ())
            .samples(100)
            .inputs(&inputs)
            .trial(|(): &(), input| always_succeeds(input))
            .baseline_dir(dir.path())
            .build()
            .run();

        // Tamper with the baseline
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml") {
                let content = std::fs::read_to_string(&path).unwrap();
                let tampered = content.replace("minPassRate: ", "minPassRate: 0.1\n# was: ");
                std::fs::write(&path, tampered).unwrap();
            }
        }

        let resolver = crate::spec::SpecResolver::with_dir(dir.path());
        // Sample-size-first needs a baseline — this path must also panic
        ProbabilisticTestBuilder::new("integrity-simple", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 50,
                confidence: 0.95,
            })
            .spec_resolver(resolver)
            .run();
    }

    // --- Feasibility scope (Change 1) ---

    #[test]
    #[should_panic(expected = "Infeasible")]
    fn verification_empirical_panics_on_infeasible() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 5,
                min_pass_rate: 0.95,
            })
            .intent(TestIntent::Verification)
            .threshold_origin(ThresholdOrigin::Empirical)
            .run();
    }

    #[test]
    #[should_panic(expected = "Infeasible")]
    fn verification_unspecified_panics_on_infeasible() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 5,
                min_pass_rate: 0.95,
            })
            .intent(TestIntent::Verification)
            .threshold_origin(ThresholdOrigin::Unspecified)
            .run();
    }

    #[test]
    fn smoke_empirical_warns_on_infeasible() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 5,
                min_pass_rate: 0.95,
            })
            .intent(TestIntent::Smoke)
            .threshold_origin(ThresholdOrigin::Empirical)
            .run();

        let warnings = result.verdict_record().warnings();
        assert!(warnings.iter().any(|w| w.code() == "UNDERSIZED"));
    }

    #[test]
    fn smoke_normative_warns_on_infeasible() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 5,
                min_pass_rate: 0.95,
            })
            .intent(TestIntent::Smoke)
            .threshold_origin(ThresholdOrigin::Sla)
            .run();

        let warnings = result.verdict_record().warnings();
        assert!(warnings.iter().any(|w| w.code() == "UNDERSIZED"));
    }

    #[test]
    fn feasible_config_no_undersized_warning() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.90,
            })
            .run();

        let warnings = result.verdict_record().warnings();
        assert!(
            !warnings.iter().any(|w| w.code() == "UNDERSIZED"),
            "should not have UNDERSIZED warning: {warnings:?}"
        );
    }

    // --- Verdict edge cases ---

    #[test]
    fn all_failures_produces_fail() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, |_| {
            TrialOutcome::failure(
                crate::model::ContractViolation::new("check", "forced"),
                Duration::from_millis(1),
            )
        })
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 50,
            min_pass_rate: 0.50,
        })
        .run();

        assert_eq!(result.verdict_record().verdict(), Verdict::Fail);
    }

    #[test]
    fn verdict_record_has_warnings() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 5,
                min_pass_rate: 0.95,
            })
            .intent(TestIntent::Smoke)
            .run();

        assert!(!result.verdict_record().warnings().is_empty());
    }

    // --- Budget exhaustion policy ---

    fn slow_success(_input: &str) -> TrialOutcome {
        std::thread::sleep(Duration::from_millis(5));
        TrialOutcome::success(Duration::from_millis(5))
    }

    fn warning_with_code<'a>(record: &'a VerdictRecord, code: &str) -> Option<&'a Warning> {
        record.warnings().iter().find(|w| w.code() == code)
    }

    #[test]
    fn time_budget_fail_policy_forces_verdict_fail() {
        let inputs = vec!["input".to_string()];
        let config = ExecutionConfig::new(100)
            .with_time_budget(Duration::from_millis(20))
            .with_on_budget_exhausted(BudgetExhaustedBehavior::Fail);

        let result = ProbabilisticTestBuilder::new("time-fail", &inputs, slow_success)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.10,
            })
            .execution_config(config)
            .run();

        let record = result.verdict_record();
        assert_eq!(record.verdict(), Verdict::Fail);
        let w = warning_with_code(record, "BUDGET_EXHAUSTED").expect("BUDGET_EXHAUSTED warning");
        let msg = w.message();
        assert!(msg.contains("time"), "message: {msg}");
        assert!(
            msg.contains("consumed"),
            "missing consumption phrase: {msg}"
        );
        assert!(msg.contains("/100 samples"), "missing sample ratio: {msg}");
    }

    #[test]
    fn token_budget_fail_policy_forces_verdict_fail() {
        let inputs = vec!["input".to_string()];
        let config = ExecutionConfig::new(100)
            .with_static_token_charge(100)
            .with_token_budget(300)
            .with_on_budget_exhausted(BudgetExhaustedBehavior::Fail);

        let result = ProbabilisticTestBuilder::new("token-fail", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.10,
            })
            .execution_config(config)
            .run();

        let record = result.verdict_record();
        assert_eq!(record.verdict(), Verdict::Fail);
        let w = warning_with_code(record, "BUDGET_EXHAUSTED").expect("BUDGET_EXHAUSTED warning");
        let msg = w.message();
        assert!(msg.contains("token"), "message: {msg}");
        assert!(
            msg.contains("consumed"),
            "missing consumption phrase: {msg}"
        );
        assert!(
            msg.contains("of 300 tokens"),
            "missing budget ceiling: {msg}"
        );
        assert!(msg.contains("/100 samples"), "missing sample ratio: {msg}");
    }

    #[test]
    fn time_budget_evaluate_partial_preserves_stats_verdict() {
        let inputs = vec!["input".to_string()];
        let config = ExecutionConfig::new(100)
            .with_time_budget(Duration::from_millis(20))
            .with_on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial);

        let result = ProbabilisticTestBuilder::new("time-partial", &inputs, slow_success)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.10,
            })
            .execution_config(config)
            .run();

        let record = result.verdict_record();
        // With pass rate 100% against threshold 0.10, the partial result passes.
        assert_eq!(record.verdict(), Verdict::Pass);
        let w = warning_with_code(record, "BUDGET_EXHAUSTED_PARTIAL")
            .expect("BUDGET_EXHAUSTED_PARTIAL warning");
        let msg = w.message();
        assert!(msg.contains("time"), "message: {msg}");
        assert!(
            msg.contains("consumed"),
            "missing consumption phrase: {msg}"
        );
        assert!(msg.contains("/100 samples"), "missing sample ratio: {msg}");
    }

    #[test]
    fn token_budget_evaluate_partial_preserves_stats_verdict() {
        let inputs = vec!["input".to_string()];
        let config = ExecutionConfig::new(100)
            .with_static_token_charge(100)
            .with_token_budget(300)
            .with_on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial);

        let result = ProbabilisticTestBuilder::new("token-partial", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.10,
            })
            .execution_config(config)
            .run();

        let record = result.verdict_record();
        assert_eq!(record.verdict(), Verdict::Pass);
        let w = warning_with_code(record, "BUDGET_EXHAUSTED_PARTIAL")
            .expect("BUDGET_EXHAUSTED_PARTIAL warning");
        let msg = w.message();
        assert!(msg.contains("token"), "message: {msg}");
        assert!(
            msg.contains("consumed"),
            "missing consumption phrase: {msg}"
        );
        assert!(
            msg.contains("of 300 tokens"),
            "missing budget ceiling: {msg}"
        );
        assert!(msg.contains("/100 samples"), "missing sample ratio: {msg}");
    }

    #[test]
    fn zero_samples_forces_fail_regardless_of_policy() {
        // A time budget of one nanosecond is exhausted by the cycles spent
        // between the Instant::now() reading and the first pre-sample check.
        // EvaluatePartial is chosen to prove the zero-samples rule overrides
        // the policy, not the policy doing the work.
        let inputs = vec!["input".to_string()];
        let config = ExecutionConfig::new(100)
            .with_time_budget(Duration::from_nanos(1))
            .with_on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial);

        let result = ProbabilisticTestBuilder::new("zero-samples", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.10,
            })
            .execution_config(config)
            .run();

        let record = result.verdict_record();
        assert_eq!(record.verdict(), Verdict::Fail);
        let w = warning_with_code(record, "BUDGET_EXHAUSTED_NO_SAMPLES").unwrap_or_else(|| {
            panic!(
                "expected BUDGET_EXHAUSTED_NO_SAMPLES warning, got {:?}",
                record.warnings()
            )
        });
        let msg = w.message();
        assert!(
            msg.contains("consumed"),
            "missing consumption phrase: {msg}"
        );
        assert!(msg.contains("of 1ns"), "missing budget ceiling: {msg}");
    }

    #[test]
    fn first_exhausted_wins_time_over_token() {
        // Time budget is tight; token budget is loose enough to not fire.
        // Warning must name "time", not "token".
        let inputs = vec!["input".to_string()];
        let config = ExecutionConfig::new(100)
            .with_time_budget(Duration::from_millis(20))
            .with_static_token_charge(100)
            .with_token_budget(100_000)
            .with_on_budget_exhausted(BudgetExhaustedBehavior::Fail);

        let result = ProbabilisticTestBuilder::new("time-wins", &inputs, slow_success)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.10,
            })
            .execution_config(config)
            .run();

        let record = result.verdict_record();
        let w = warning_with_code(record, "BUDGET_EXHAUSTED").expect("BUDGET_EXHAUSTED warning");
        let msg = w.message();
        assert!(msg.contains("time"), "message: {msg}");
        // "token" appears nowhere — not as the budget name, and not as a unit
        // since the time path formats Duration rather than a tokens count.
        assert!(!msg.contains("token"), "message: {msg}");
        assert!(
            msg.contains("consumed"),
            "missing consumption phrase: {msg}"
        );
        assert!(msg.contains("/100 samples"), "missing sample ratio: {msg}");
    }

    // --- Isolated tests for extracted helpers ---

    fn make_summary(
        reason: TerminationReason,
        samples_executed: u32,
        samples_planned: u32,
        time_consumed: Duration,
        tokens_consumed: u64,
    ) -> ExecutionSummary {
        use crate::model::{CostSummary, TerminationInfo};
        ExecutionSummary::new(
            samples_planned,
            samples_executed,
            samples_executed,
            0,
            TerminationInfo::new(reason),
            CostSummary::new(time_consumed, tokens_consumed, samples_executed),
        )
    }

    fn make_summary_with_run_snapshot(
        reason: TerminationReason,
        samples_executed: u32,
        samples_planned: u32,
        time_consumed: Duration,
        tokens_consumed: u64,
        snapshot: crate::model::RunScopedSnapshot,
    ) -> ExecutionSummary {
        use crate::model::{CostSummary, TerminationInfo};
        let cost = CostSummary::new(time_consumed, tokens_consumed, samples_executed)
            .with_run_scoped(snapshot);
        ExecutionSummary::new(
            samples_planned,
            samples_executed,
            samples_executed,
            0,
            TerminationInfo::new(reason),
            cost,
        )
    }

    #[test]
    fn apply_budget_exhaustion_policy_fail_forces_verdict_fail() {
        let summary = make_summary(
            TerminationReason::TimeBudgetExhausted,
            5,
            100,
            Duration::from_millis(25),
            0,
        );
        let config = ExecutionConfig::new(100)
            .with_time_budget(Duration::from_millis(20))
            .with_on_budget_exhausted(BudgetExhaustedBehavior::Fail);
        let mut warnings = Vec::new();

        let verdict =
            apply_budget_exhaustion_policy(&summary, &config, Verdict::Pass, &mut warnings);

        assert_eq!(verdict, Verdict::Fail);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "BUDGET_EXHAUSTED");
    }

    #[test]
    fn apply_budget_exhaustion_policy_partial_preserves_verdict() {
        let summary = make_summary(
            TerminationReason::TokenBudgetExhausted,
            3,
            100,
            Duration::ZERO,
            400,
        );
        let config = ExecutionConfig::new(100)
            .with_static_token_charge(100)
            .with_token_budget(300)
            .with_on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial);
        let mut warnings = Vec::new();

        let verdict =
            apply_budget_exhaustion_policy(&summary, &config, Verdict::Pass, &mut warnings);

        assert_eq!(verdict, Verdict::Pass);
        assert_eq!(warnings[0].code(), "BUDGET_EXHAUSTED_PARTIAL");
    }

    #[test]
    fn apply_budget_exhaustion_policy_zero_samples_always_fails() {
        let summary = make_summary(
            TerminationReason::TimeBudgetExhausted,
            0,
            100,
            Duration::from_nanos(1),
            0,
        );
        let config = ExecutionConfig::new(100)
            .with_time_budget(Duration::from_nanos(1))
            .with_on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial);
        let mut warnings = Vec::new();

        let verdict =
            apply_budget_exhaustion_policy(&summary, &config, Verdict::Pass, &mut warnings);

        assert_eq!(verdict, Verdict::Fail);
        assert_eq!(warnings[0].code(), "BUDGET_EXHAUSTED_NO_SAMPLES");
    }

    #[test]
    fn apply_budget_exhaustion_policy_non_budget_termination_passes_through() {
        let summary = make_summary(
            TerminationReason::Completed,
            100,
            100,
            Duration::from_millis(10),
            0,
        );
        let config = ExecutionConfig::new(100);
        let mut warnings = Vec::new();

        let verdict =
            apply_budget_exhaustion_policy(&summary, &config, Verdict::Pass, &mut warnings);

        assert_eq!(verdict, Verdict::Pass);
        assert!(warnings.is_empty());
    }

    // --- enforce_feasibility ---

    fn feasible_result() -> FeasibilityResult {
        use crate::statistics::types::ConfidenceLevel;
        feasibility::feasibility_check(1000, 0.50, ConfidenceLevel::new(0.95))
    }

    fn infeasible_result() -> FeasibilityResult {
        use crate::statistics::types::ConfidenceLevel;
        // 2 samples against a 99% target at 99% confidence is nowhere near
        // enough to prove the proportion; Wilson lower-bound machinery
        // reports this as infeasible.
        feasibility::feasibility_check(2, 0.99, ConfidenceLevel::new(0.99))
    }

    #[test]
    fn enforce_feasibility_feasible_is_no_op() {
        let feas = feasible_result();
        assert!(feas.feasible());
        let mut warnings = Vec::new();
        enforce_feasibility("uc", TestIntent::Verification, &feas, &mut warnings);
        assert!(warnings.is_empty());
    }

    #[test]
    fn enforce_feasibility_smoke_infeasible_warns_undersized() {
        let feas = infeasible_result();
        assert!(!feas.feasible());
        let mut warnings = Vec::new();
        enforce_feasibility("uc", TestIntent::Smoke, &feas, &mut warnings);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "UNDERSIZED");
    }

    #[test]
    #[should_panic(expected = "Infeasible")]
    fn enforce_feasibility_verification_infeasible_panics() {
        let feas = infeasible_result();
        let mut warnings = Vec::new();
        enforce_feasibility("uc", TestIntent::Verification, &feas, &mut warnings);
    }

    // --- compute_stats_verdict ---

    fn make_derived_threshold(value: f64) -> DerivedThreshold {
        use crate::statistics::types::{ConfidenceLevel, DerivationContext, OperationalApproach};
        let cl = ConfidenceLevel::new(0.95);
        let ctx = DerivationContext::new(0.9, 100, 100, cl);
        DerivedThreshold::new(value, OperationalApproach::SampleSizeFirst, ctx, true)
    }

    #[test]
    fn compute_stats_verdict_zero_samples_fails() {
        let summary = make_summary(TerminationReason::Completed, 0, 100, Duration::ZERO, 0);
        let threshold = make_derived_threshold(0.50);
        assert_eq!(compute_stats_verdict(&summary, &threshold), Verdict::Fail);
    }

    #[test]
    fn compute_stats_verdict_passes_when_stats_pass() {
        // 100 successes out of 100 against a 0.10 threshold is clearly a pass.
        let summary = make_summary(
            TerminationReason::Completed,
            100,
            100,
            Duration::from_millis(10),
            0,
        );
        let threshold = make_derived_threshold(0.10);
        assert_eq!(compute_stats_verdict(&summary, &threshold), Verdict::Pass);
    }

    #[test]
    fn compute_stats_verdict_fails_when_stats_fail() {
        // 10 successes out of 50 (20% observed) against a 0.95 threshold:
        // the z-test rejects the null, so the verdict is Fail.
        use crate::model::{CostSummary, TerminationInfo};
        let summary = ExecutionSummary::new(
            50,
            50,
            10,
            40,
            TerminationInfo::new(TerminationReason::Completed),
            CostSummary::new(Duration::ZERO, 0, 50),
        );
        let threshold = make_derived_threshold(0.95);
        assert_eq!(compute_stats_verdict(&summary, &threshold), Verdict::Fail);
    }

    // --- apply_expiration_policy ---

    fn make_expiration(status: crate::model::ExpirationStatus) -> ExpirationInfo {
        ExpirationInfo::new(status, Some("2026-01-01T00:00:00Z".into()))
    }

    #[test]
    fn apply_expiration_policy_none_info_passes_through() {
        let mut warnings = Vec::new();
        let verdict = apply_expiration_policy(None, true, Verdict::Pass, &mut warnings);
        assert_eq!(verdict, Verdict::Pass);
        assert!(warnings.is_empty());
    }

    #[test]
    fn apply_expiration_policy_not_expired_passes_through() {
        let info = make_expiration(crate::model::ExpirationStatus::Valid);
        let mut warnings = Vec::new();
        let verdict = apply_expiration_policy(Some(&info), true, Verdict::Pass, &mut warnings);
        assert_eq!(verdict, Verdict::Pass);
        assert!(warnings.is_empty());
    }

    #[test]
    fn apply_expiration_policy_expired_warn_only_preserves_verdict() {
        let info = make_expiration(crate::model::ExpirationStatus::Expired);
        let mut warnings = Vec::new();
        let verdict = apply_expiration_policy(Some(&info), false, Verdict::Pass, &mut warnings);
        assert_eq!(verdict, Verdict::Pass);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "BASELINE_EXPIRED");
        assert!(warnings[0].message().contains("re-run"));
    }

    #[test]
    fn apply_expiration_policy_expired_fail_forces_fail() {
        let info = make_expiration(crate::model::ExpirationStatus::Expired);
        let mut warnings = Vec::new();
        let verdict = apply_expiration_policy(Some(&info), true, Verdict::Pass, &mut warnings);
        assert_eq!(verdict, Verdict::Fail);
        assert_eq!(warnings[0].code(), "BASELINE_EXPIRED");
        assert!(warnings[0].message().contains("failing per"));
    }

    // --- record_smoke_normative_warning ---

    fn make_criteria(intent: TestIntent, origin: ThresholdOrigin) -> AssessmentCriteria {
        AssessmentCriteria {
            approach: ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.90,
            },
            intent,
            threshold_origin: origin,
            contract_ref: None,
            latency: LatencyConfig::default(),
            fail_on_expired_baseline: false,
            on_budget_exhausted: None,
        }
    }

    #[test]
    fn record_smoke_normative_warning_fires_on_smoke_plus_normative() {
        let criteria = make_criteria(TestIntent::Smoke, ThresholdOrigin::Sla);
        let mut warnings = Vec::new();
        record_smoke_normative_warning(&criteria, &mut warnings);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "SMOKE_NORMATIVE");
    }

    #[test]
    fn record_smoke_normative_warning_silent_on_verification() {
        let criteria = make_criteria(TestIntent::Verification, ThresholdOrigin::Sla);
        let mut warnings = Vec::new();
        record_smoke_normative_warning(&criteria, &mut warnings);
        assert!(warnings.is_empty());
    }

    #[test]
    fn record_smoke_normative_warning_silent_on_non_normative_origin() {
        let criteria = make_criteria(TestIntent::Smoke, ThresholdOrigin::Empirical);
        let mut warnings = Vec::new();
        record_smoke_normative_warning(&criteria, &mut warnings);
        assert!(warnings.is_empty());
    }

    // --- consumption_phrase ---

    #[test]
    fn consumption_phrase_time_reports_duration() {
        let summary = make_summary(
            TerminationReason::TimeBudgetExhausted,
            5,
            100,
            Duration::from_millis(23),
            0,
        );
        let config = ExecutionConfig::new(100).with_time_budget(Duration::from_millis(20));
        let phrase = consumption_phrase(summary.termination().reason(), &summary, &config);
        assert!(phrase.contains("23ms"), "phrase: {phrase}");
        assert!(phrase.contains("of 20ms"), "phrase: {phrase}");
    }

    #[test]
    fn consumption_phrase_token_reports_tokens() {
        let summary = make_summary(
            TerminationReason::TokenBudgetExhausted,
            3,
            100,
            Duration::ZERO,
            400,
        );
        let config = ExecutionConfig::new(100).with_token_budget(300);
        let phrase = consumption_phrase(summary.termination().reason(), &summary, &config);
        assert!(phrase.contains("400"), "phrase: {phrase}");
        assert!(phrase.contains("of 300 tokens"), "phrase: {phrase}");
    }

    #[test]
    fn consumption_phrase_non_budget_termination_empty() {
        let summary = make_summary(
            TerminationReason::Completed,
            100,
            100,
            Duration::from_millis(50),
            0,
        );
        let config = ExecutionConfig::new(100);
        let phrase = consumption_phrase(summary.termination().reason(), &summary, &config);
        assert!(phrase.is_empty(), "phrase: {phrase}");
    }

    #[test]
    fn consumption_phrase_run_scoped_time_reports_from_snapshot() {
        use crate::model::RunScopedSnapshot;
        let summary = make_summary_with_run_snapshot(
            TerminationReason::RunTimeBudgetExhausted,
            2,
            100,
            Duration::from_millis(7),
            0,
            RunScopedSnapshot::new(
                Some(Duration::from_secs(10)),
                Duration::from_secs(11),
                None,
                0,
            ),
        );
        let config = ExecutionConfig::new(100);
        let phrase = consumption_phrase(summary.termination().reason(), &summary, &config);
        assert!(phrase.contains("11s"), "phrase: {phrase}");
        assert!(phrase.contains("of 10s"), "phrase: {phrase}");
    }

    #[test]
    fn consumption_phrase_run_scoped_token_reports_from_snapshot() {
        use crate::model::RunScopedSnapshot;
        let summary = make_summary_with_run_snapshot(
            TerminationReason::RunTokenBudgetExhausted,
            3,
            100,
            Duration::ZERO,
            600,
            RunScopedSnapshot::new(None, Duration::ZERO, Some(5_000), 5_200),
        );
        let config = ExecutionConfig::new(100);
        let phrase = consumption_phrase(summary.termination().reason(), &summary, &config);
        assert!(phrase.contains("5200"), "phrase: {phrase}");
        assert!(phrase.contains("of 5000 tokens"), "phrase: {phrase}");
    }

    #[test]
    fn apply_budget_exhaustion_policy_run_scoped_time_fail_emits_warning() {
        use crate::model::RunScopedSnapshot;
        let summary = make_summary_with_run_snapshot(
            TerminationReason::RunTimeBudgetExhausted,
            15,
            100,
            Duration::from_millis(4),
            0,
            RunScopedSnapshot::new(
                Some(Duration::from_secs(5)),
                Duration::from_secs(6),
                None,
                0,
            ),
        );
        let config =
            ExecutionConfig::new(100).with_on_budget_exhausted(BudgetExhaustedBehavior::Fail);
        let mut warnings = Vec::new();

        let verdict =
            apply_budget_exhaustion_policy(&summary, &config, Verdict::Pass, &mut warnings);

        assert_eq!(verdict, Verdict::Fail);
        assert_eq!(warnings[0].code(), "BUDGET_EXHAUSTED");
        let msg = warnings[0].message();
        assert!(
            msg.contains("run-scoped time budget exhausted"),
            "message: {msg}"
        );
        assert!(msg.contains("/100 samples"), "missing sample ratio: {msg}");
        assert!(msg.contains("of 5s"), "missing consumption: {msg}");
    }

    #[test]
    fn apply_budget_exhaustion_policy_run_scoped_tokens_partial_preserves_verdict() {
        use crate::model::RunScopedSnapshot;
        let summary = make_summary_with_run_snapshot(
            TerminationReason::RunTokenBudgetExhausted,
            20,
            100,
            Duration::ZERO,
            2_000,
            RunScopedSnapshot::new(None, Duration::ZERO, Some(10_000), 10_400),
        );
        let config = ExecutionConfig::new(100)
            .with_on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial);
        let mut warnings = Vec::new();

        let verdict =
            apply_budget_exhaustion_policy(&summary, &config, Verdict::Pass, &mut warnings);

        assert_eq!(verdict, Verdict::Pass);
        assert_eq!(warnings[0].code(), "BUDGET_EXHAUSTED_PARTIAL");
        let msg = warnings[0].message();
        assert!(
            msg.contains("run-scoped token budget exhausted"),
            "message: {msg}"
        );
        assert!(msg.contains("20/100 samples"), "sample ratio wrong: {msg}");
        assert!(msg.contains("of 10000 tokens"), "consumption wrong: {msg}");
    }

    #[test]
    fn apply_budget_exhaustion_policy_run_scoped_zero_samples_fails() {
        use crate::model::RunScopedSnapshot;
        let summary = make_summary_with_run_snapshot(
            TerminationReason::RunTokenBudgetExhausted,
            0,
            50,
            Duration::ZERO,
            0,
            RunScopedSnapshot::new(None, Duration::ZERO, Some(1_000), 1_000),
        );
        let config = ExecutionConfig::new(50)
            .with_on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial);
        let mut warnings = Vec::new();

        let verdict =
            apply_budget_exhaustion_policy(&summary, &config, Verdict::Pass, &mut warnings);

        assert_eq!(verdict, Verdict::Fail);
        assert_eq!(warnings[0].code(), "BUDGET_EXHAUSTED_NO_SAMPLES");
        assert!(
            warnings[0]
                .message()
                .contains("run-scoped token budget exhausted"),
            "message: {}",
            warnings[0].message()
        );
    }

    // --- synthesise_execution_config ---

    #[test]
    fn synthesise_uses_override_when_present() {
        let override_config = ExecutionConfig::new(50)
            .with_time_budget(Duration::from_secs(5))
            .with_on_budget_exhausted(BudgetExhaustedBehavior::Fail);
        let criteria = make_criteria(TestIntent::Verification, ThresholdOrigin::Empirical);
        let threshold = make_derived_threshold(0.50);
        let feas = feasible_result();

        let config = synthesise_execution_config(
            Some(&override_config),
            &criteria,
            100, // would differ from override's 50 if synthesis ignored the override
            &threshold,
            &feas,
        );

        assert_eq!(config.samples(), 50);
        assert_eq!(config.time_budget(), Some(Duration::from_secs(5)));
        assert_eq!(config.on_budget_exhausted(), BudgetExhaustedBehavior::Fail);
    }

    #[test]
    fn synthesise_applies_criteria_on_budget_exhausted_when_no_override() {
        let mut criteria = make_criteria(TestIntent::Verification, ThresholdOrigin::Empirical);
        criteria.on_budget_exhausted = Some(BudgetExhaustedBehavior::EvaluatePartial);
        let threshold = make_derived_threshold(0.50);
        let feas = feasible_result();

        let config = synthesise_execution_config(None, &criteria, 100, &threshold, &feas);

        assert_eq!(config.samples(), 100);
        assert_eq!(
            config.on_budget_exhausted(),
            BudgetExhaustedBehavior::EvaluatePartial
        );
    }

    #[test]
    fn synthesise_override_wins_over_criteria_on_budget_exhausted() {
        let override_config =
            ExecutionConfig::new(100).with_on_budget_exhausted(BudgetExhaustedBehavior::Fail);
        let mut criteria = make_criteria(TestIntent::Verification, ThresholdOrigin::Empirical);
        criteria.on_budget_exhausted = Some(BudgetExhaustedBehavior::EvaluatePartial);
        let threshold = make_derived_threshold(0.50);
        let feas = feasible_result();

        let config =
            synthesise_execution_config(Some(&override_config), &criteria, 100, &threshold, &feas);

        // Override wins.
        assert_eq!(config.on_budget_exhausted(), BudgetExhaustedBehavior::Fail);
    }
}
