//! Probabilistic test execution and verdict production.

use crate::controls::{Cost, ExecutionConfig, TokenRecorder};
use crate::criteria::{Criteria, CriteriaCounts, CriterionCounts, CriterionTarget};
use crate::experiment::{ContractExecutionResult, ExecutionEngine, SampleEvaluation};
use crate::latency::{
    LatencyCriterion, LatencyDimension, LatencyEnforcementMode, LatencyThresholds, enforcement,
    resolver,
};
use crate::model::{
    BudgetExhaustedBehavior, ExecutionSummary, ExpirationInfo, PacingSummary, TerminationReason,
    TestIdentity, TestIntent, ThresholdOrigin, Warning,
};
use crate::ptest::approach;
use crate::ptest::builder::ThresholdApproach;
use crate::ptest::diagnostics;
use crate::service_contract::CovariateContext;
use crate::service_contract::ServiceContract;
use crate::spec::{BaselineSpec, SpecResolver};
use crate::statistics::types::{ConfidenceLevel, DerivationContext, OperationalApproach};
use crate::statistics::types::{DerivedThreshold, FeasibilityResult};
use crate::statistics::{evaluator, feasibility, proportion, threshold};
use crate::verdict::{
    BaselineProvenance, CriterionRow, FunctionalAssessment, SpecProvenance, StatisticalAnalysis,
    Verdict, VerdictRecord,
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
    /// When true, the per-sample early-termination check is disabled: every
    /// declared sample runs even once the verdict is determined. The runner
    /// then leaves `min_pass_rate` / `min_samples_for_validity` unset, so the
    /// engine reports `TerminationReason::Completed`.
    pub early_termination_disabled: bool,
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

/// Resolves the baseline spec via the context's pre-resolved slot or
/// its resolver, whichever is set. Warnings from the resolution path
/// are pushed into the caller's vec.
fn resolve_baseline(
    baseline: BaselineContext,
    service_contract_id: &str,
    warnings: &mut Vec<Warning>,
) -> Option<BaselineSpec> {
    baseline.pre_resolved_spec.or_else(|| {
        baseline.spec_resolver.as_ref().and_then(|resolver| {
            crate::ptest::baseline::resolve(
                resolver,
                service_contract_id,
                baseline.covariate_context.as_ref(),
                warnings,
            )
        })
    })
}

/// Panics (under `Verification` intent) or warns (under `Smoke` intent)
/// when the configuration is statistically infeasible.
fn enforce_feasibility(
    service_contract_id: &str,
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
                diagnostics::infeasibility_message(service_contract_id, feas, false),
            );
        }
        TestIntent::Smoke => {
            warnings.push(Warning::new(
                "UNDERSIZED",
                diagnostics::infeasibility_message(service_contract_id, feas, false),
            ));
        }
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
        format!("{}.yaml", spec.service_contract_id),
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

/// Builds spec provenance from the baseline spec and contract ref.
fn build_provenance(
    threshold_origin: ThresholdOrigin,
    baseline_spec: Option<&crate::spec::BaselineSpec>,
    contract_ref: Option<&str>,
    expiration_info: Option<crate::model::ExpirationInfo>,
) -> SpecProvenance {
    let mut provenance = SpecProvenance::new(threshold_origin);
    if let Some(spec) = baseline_spec {
        provenance = provenance.with_spec_filename(format!("{}.yaml", spec.service_contract_id));
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

/// Executes a contract-driven probabilistic test: the engine invokes the
/// contract and judges every criterion on every sample, and the verdict
/// decomposes per criterion with a composite over them.
///
/// Mirrors [`execute`] — baseline resolution, threshold derivation, feasibility
/// enforcement, budget/expiration policy, and provenance are identical — but
/// drives the fused [`ServiceContract`] (`invoke` → `criteria().evaluate`)
/// instead of a trial closure, builds a per-criterion composite verdict, and
/// sources latency commitments from [`ServiceContract::latency`].
///
/// # Panics
///
/// Panics if `inputs` is empty, if the configuration is statistically
/// infeasible under verification intent, or if a service invocation yields a
/// defect (a transport failure or a caught panic) — a defect aborts the run.
pub fn execute_contract<C: ServiceContract>(
    contract: &C,
    inputs: &[C::Input],
    criteria: &AssessmentCriteria,
    baseline: BaselineContext,
    config_overrides: Option<&ExecutionConfig>,
) -> ProbabilisticTestResult
where
    C::Output: 'static,
{
    assert!(
        !inputs.is_empty(),
        "a probabilistic test requires at least one input"
    );

    let mut warnings: Vec<Warning> = Vec::new();
    let service_contract_id = contract.id().to_owned();

    let baseline_spec = resolve_baseline(baseline, &service_contract_id, &mut warnings);

    // The contract's criteria are resolved before the threshold: risk-driven
    // sizing computes the governing sample count from each baseline-derived
    // criterion's own baseline tally.
    let contract_criteria = contract.criteria();
    let (samples, derived_threshold, resolved_confidence, feas) = resolve_sampling_plan(
        &criteria.approach,
        baseline_spec.as_ref(),
        &contract_criteria.targets(),
    );
    enforce_feasibility(&service_contract_id, criteria.intent, &feas, &mut warnings);

    let config = resolve_execution_config(
        config_overrides,
        samples,
        criteria,
        &derived_threshold,
        &feas,
        contract.warmup(),
    );

    let token_recorder = TokenRecorder::new();
    let exec_result = run_contract_sampling(
        contract,
        inputs,
        &config,
        &contract_criteria,
        &token_recorder,
    );

    let summary = exec_result.summary();

    let rows = build_criterion_rows(
        exec_result.criteria_counts(),
        &contract_criteria.targets(),
        resolved_confidence,
        baseline_spec.as_ref(),
        criteria.threshold_origin,
        criteria.intent,
    );
    let analysis = rows
        .first()
        .and_then(|row| row.statistical_analysis().cloned());

    // Budget exhaustion and baseline expiration adjust the overall (composite)
    // verdict, exactly as for the legacy single-criterion path.
    let mut verdict = composite_verdict(&rows);
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

    let assessment = FunctionalAssessment::new(verdict, rows);

    let latency_dimension = build_contract_latency_dimension(
        contract.latency(),
        &criteria.latency,
        exec_result.aggregate().successful_latencies(),
        baseline_spec.as_ref(),
        &mut warnings,
    );

    let provenance = build_provenance(
        criteria.threshold_origin,
        baseline_spec.as_ref(),
        criteria.contract_ref.as_deref(),
        expiration_info,
    );
    let baseline_prov = baseline_spec.as_ref().map(build_baseline_provenance);

    let mut builder = VerdictRecord::builder(
        TestIdentity::new(service_contract_id),
        verdict,
        criteria.intent,
        summary.clone(),
        assessment,
    )
    .spec_provenance(provenance);
    if let Some(analysis) = analysis {
        builder = builder.statistical_analysis(analysis);
    }
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

/// Resolves the sampling plan for a run: the sample count, the derived
/// threshold, the resolved confidence, and the feasibility check over them.
/// Risk-driven plans size against the per-criterion baseline tallies of the
/// contract's baseline-derived criteria (see [`empirical_criterion_tallies`]).
fn resolve_sampling_plan(
    approach: &ThresholdApproach,
    baseline_spec: Option<&BaselineSpec>,
    targets: &[(&str, &CriterionTarget)],
) -> (u32, DerivedThreshold, ConfidenceLevel, FeasibilityResult) {
    let criterion_tallies = empirical_criterion_tallies(targets, baseline_spec);
    let (samples, derived_threshold) = approach::resolve_threshold(
        approach,
        baseline_spec.map(|s| &s.statistics),
        baseline_spec.map(|s| &s.execution),
        &criterion_tallies,
    );
    let resolved_confidence = approach::resolved_confidence(approach);
    let feas =
        feasibility::feasibility_check(samples, derived_threshold.value(), resolved_confidence);
    (samples, derived_threshold, resolved_confidence, feas)
}

/// Synthesises the execution config for a run: an explicit caller override is
/// used as-is, otherwise one is built from the resolved sample size and the
/// criteria's budget-exhaustion behaviour. Early termination is wired in only
/// under a meaningful threshold — a `0.0` floor (the bare-samples plan) runs
/// every sample and lets the per-criterion targets decide the verdict.
fn resolve_execution_config(
    config_overrides: Option<&ExecutionConfig>,
    samples: u32,
    criteria: &AssessmentCriteria,
    derived_threshold: &DerivedThreshold,
    feas: &FeasibilityResult,
    warmup: u32,
) -> ExecutionConfig {
    let mut config = config_overrides.cloned().unwrap_or_else(|| {
        let mut c = ExecutionConfig::new(samples);
        if let Some(behaviour) = criteria.on_budget_exhausted {
            c = c.with_on_budget_exhausted(behaviour);
        }
        c
    });
    if derived_threshold.value() > 0.0 && !criteria.early_termination_disabled {
        config = config
            .min_pass_rate(derived_threshold.value())
            .min_samples_for_validity(feas.minimum_samples());
    }
    config.with_warmup(warmup)
}

/// Runs the sampling loop for `contract`, timing each invocation and recording
/// its token cost, then evaluating the output against `contract_criteria`.
///
/// # Panics
///
/// Panics if a service invocation yields a defect (a transport failure or a
/// caught panic) — a defect aborts the run.
fn run_contract_sampling<C: ServiceContract>(
    contract: &C,
    inputs: &[C::Input],
    config: &ExecutionConfig,
    contract_criteria: &Criteria<C::Output>,
    token_recorder: &TokenRecorder,
) -> ContractExecutionResult
where
    C::Output: 'static,
{
    let recorder = token_recorder.clone();
    ExecutionEngine::run_contract(
        config,
        inputs,
        token_recorder,
        crate::controls::run::current(),
        |input: &C::Input| {
            let mut cost = Cost::new();
            let start = std::time::Instant::now();
            let output = contract.invoke(input, &mut cost)?;
            let elapsed = start.elapsed();
            recorder.record(cost.tokens_recorded());
            let expected = contract.expected(input);
            Ok(SampleEvaluation {
                results: contract_criteria.evaluate(&output, expected.as_ref()),
                elapsed,
            })
        },
    )
    .unwrap_or_else(|defect| {
        panic!("\n\nservice invocation aborted the run: {defect}\n");
    })
}

/// Builds one verdict row per criterion, each judged against its own target.
fn build_criterion_rows(
    counts: &CriteriaCounts,
    targets: &[(&str, &CriterionTarget)],
    confidence: ConfidenceLevel,
    baseline: Option<&BaselineSpec>,
    threshold_origin: ThresholdOrigin,
    intent: TestIntent,
) -> Vec<CriterionRow> {
    targets
        .iter()
        .map(|(name, target)| {
            build_criterion_row(
                name,
                target,
                counts,
                confidence,
                baseline,
                threshold_origin,
                intent,
            )
        })
        .collect()
}

/// The composite verdict over the criterion rows: `Inconclusive` if any row is,
/// otherwise the conjunction (`Pass` only if every row passed).
fn composite_verdict(rows: &[CriterionRow]) -> Verdict {
    if rows.iter().any(|r| r.verdict() == Verdict::Inconclusive) {
        Verdict::Inconclusive
    } else if rows.iter().all(|r| r.verdict() == Verdict::Pass) {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

/// Builds one criterion's verdict row. A criterion with no in-scope trials, or
/// one whose feasibility gate fails, is `Inconclusive` (verdict-level only). A
/// zero-failures criterion is observational — `Pass` iff it recorded no
/// failures. Otherwise the criterion is judged posture-explicitly (see
/// [`criterion_meets_target`]): a normative target by the tally's own Wilson
/// lower bound clearing the declared rate, an empirical target by the observed
/// success count meeting the derived integer cutoff.
fn build_criterion_row(
    name: &str,
    target: &CriterionTarget,
    counts: &CriteriaCounts,
    confidence: ConfidenceLevel,
    baseline: Option<&BaselineSpec>,
    threshold_origin: ThresholdOrigin,
    intent: TestIntent,
) -> CriterionRow {
    let tally = counts.get(name);
    let pass = tally.map_or(0, CriterionCounts::pass);
    let fail = tally.map_or(0, CriterionCounts::fail);
    let distribution: Vec<(String, u32)> = tally.map_or_else(Vec::new, |t: &CriterionCounts| {
        t.failure_distribution()
            .iter()
            .map(|(check, count)| (check.clone(), *count))
            .collect()
    });

    let total = pass + fail;
    if total == 0 {
        return CriterionRow::new(name, pass, fail, distribution, None, Verdict::Inconclusive);
    }

    if matches!(target, CriterionTarget::ZeroFailures) {
        let verdict = if fail == 0 {
            Verdict::Pass
        } else {
            Verdict::Fail
        };
        return CriterionRow::new(name, pass, fail, distribution, None, verdict);
    }

    let derived = match target {
        CriterionTarget::NormativeRate(rate) => {
            let context = DerivationContext::new(*rate, total, total, confidence);
            DerivedThreshold::new(*rate, OperationalApproach::ThresholdFirst, context, false)
        }
        CriterionTarget::EmpiricalRate => {
            let (baseline_successes, baseline_samples) = criterion_baseline(name, baseline);
            threshold::derive_sample_size_first(
                baseline_successes,
                baseline_samples,
                total,
                confidence,
            )
        }
        CriterionTarget::ZeroFailures => unreachable!("handled above"),
    };

    // A smoke test opts into the sizing gap: it skips the feasibility gate and
    // renders the nominal verdict. A verification test that cannot be sized to
    // confirm its target is verdict-level Inconclusive.
    if intent == TestIntent::Verification
        && !feasibility::feasibility_check(total, derived.value(), confidence).feasible()
    {
        return CriterionRow::new(name, pass, fail, distribution, None, Verdict::Inconclusive);
    }

    let verdict = if criterion_meets_target(pass, total, target, &derived, confidence) {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    let analysis = criterion_analysis(pass, total, &derived, threshold_origin);
    CriterionRow::new(name, pass, fail, distribution, Some(analysis), verdict)
}

/// Judges one criterion's tally against its target, posture-explicit.
///
/// A normative (declared) rate demands the tally's own Wilson lower bound
/// clear it — the compliance posture, where the test sample carries its own
/// sampling uncertainty. An empirical (baseline-derived) threshold decides on
/// the derived integer cutoff, pass iff the success count meets it — the
/// regression posture, where the uncertainty was priced into the derivation.
fn criterion_meets_target(
    pass: u32,
    total: u32,
    target: &CriterionTarget,
    derived: &DerivedThreshold,
    confidence: ConfidenceLevel,
) -> bool {
    match target {
        CriterionTarget::NormativeRate(rate) => {
            evaluator::meets_declared_rate(pass, total, *rate, confidence)
        }
        CriterionTarget::EmpiricalRate => {
            let cutoff = derived
                .decision_cutoff()
                .expect("a sample-size-first derivation carries its decision cutoff")
                .cutoff();
            pass >= cutoff
        }
        CriterionTarget::ZeroFailures => unreachable!("judged before threshold derivation"),
    }
}

/// Resolves the baseline successes and sample count for an empirical criterion,
/// preferring its own per-criterion measurement and falling back to the
/// whole-contract aggregate when the baseline predates per-criterion capture.
///
/// # Panics
///
/// Panics if no baseline is available — an empirical criterion requires one.
fn criterion_baseline(name: &str, baseline: Option<&BaselineSpec>) -> (u32, u32) {
    let spec = baseline.expect("an empirical criterion requires a baseline");
    spec.statistics
        .per_criterion
        .as_ref()
        .and_then(|per| per.get(name))
        .map_or(
            (spec.statistics.successes, spec.execution.samples_executed),
            |criterion| {
                (
                    criterion.successes,
                    criterion.successes + criterion.failures,
                )
            },
        )
}

/// Resolves each baseline-derived criterion's baseline tally for risk-driven
/// sizing, applying exactly the per-criterion resolution (and whole-contract
/// aggregate fallback) of [`criterion_baseline`]. Returns an empty vector when
/// no baseline is available — the approaches that need one panic on their own
/// terms during threshold resolution.
fn empirical_criterion_tallies(
    targets: &[(&str, &CriterionTarget)],
    baseline: Option<&BaselineSpec>,
) -> Vec<approach::CriterionBaselineTally> {
    let Some(spec) = baseline else {
        return Vec::new();
    };
    targets
        .iter()
        .filter(|(_, target)| matches!(target, CriterionTarget::EmpiricalRate))
        .map(|(name, _)| {
            let (successes, trials) = criterion_baseline(name, Some(spec));
            approach::CriterionBaselineTally {
                criterion_name: (*name).to_owned(),
                successes,
                trials,
            }
        })
        .collect()
}

/// Builds a criterion's statistical analysis from its observed tally against
/// its derived threshold.
fn criterion_analysis(
    pass: u32,
    total: u32,
    derived: &DerivedThreshold,
    threshold_origin: ThresholdOrigin,
) -> StatisticalAnalysis {
    let confidence = derived.context().confidence();
    let se = proportion::standard_error(pass, total);
    let wilson_lower = proportion::lower_bound(pass, total, confidence);
    let observed = f64::from(pass) / f64::from(total);
    let z = proportion::z_test_statistic(observed, derived.value(), total);
    let p = proportion::one_sided_p_value(z);
    StatisticalAnalysis::new(
        confidence.value(),
        se,
        wilson_lower,
        derived.value(),
        threshold_origin,
    )
    .with_test_results(z, p)
}

/// Builds the latency dimension for a contract-driven run, sourcing explicit
/// percentile ceilings from the contract's latency criterion. Explicit ceilings
/// are enforced strictly; a baseline latency block still contributes advisory
/// (or env-configured) derived thresholds.
fn build_contract_latency_dimension(
    latency: Option<LatencyCriterion>,
    latency_config: &LatencyConfig,
    successful_latencies: &[std::time::Duration],
    baseline_spec: Option<&BaselineSpec>,
    warnings: &mut Vec<Warning>,
) -> Option<LatencyDimension> {
    // Explicit ceilings come from the contract's latency criterion; the
    // baseline-derived enforcement mode and confidence carry over from the
    // test's latency configuration.
    let config = LatencyConfig {
        thresholds: latency.map_or_else(LatencyThresholds::new, |c| *c.thresholds()),
        baseline_mode: latency_config.baseline_mode,
        baseline_confidence: latency_config.baseline_confidence,
    };
    build_latency_dimension(&config, successful_latencies, baseline_spec, warnings)
}
