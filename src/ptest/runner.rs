//! Probabilistic test execution and verdict production.
//!
//! A run resolves its baseline and sampling plan, refuses an invalid
//! configuration whole before any sample runs (recording the refusal),
//! samples the contract, decides each criterion by its rule and each latency
//! constraint by the rule for its threshold source, and composes the test
//! verdict `V_test` from the dimensions the run enforces (Statistical
//! Companion §1.4.6, §12.6). An advisory dimension is decided exactly as an
//! enforced one and reported beside the test verdict.

use crate::controls::{Cost, ExecutionConfig, TokenRecorder};
use crate::criteria::{Criteria, CriterionTarget};
use crate::experiment::{ContractExecutionResult, ExecutionEngine, SampleEvaluation};
use crate::latency::{
    ConstraintConfidence, LatencyCriterion, LatencyDimension, ResolvedLatencyConstraint, resolver,
};
use crate::model::{
    BudgetExhaustedBehavior, CostSummary, ExecutionSummary, ExpirationInfo, PacingSummary,
    TerminationInfo, TerminationReason, TestIdentity, TestIntent, ThresholdOrigin, Warning,
};
use crate::ptest::approach::{self, CriterionBaselineTally, RunPlan};
use crate::ptest::builder::ThresholdApproach;
use crate::ptest::diagnostics;
use crate::ptest::disclosure;
use crate::ptest::judge::{self, CriterionContext};
use crate::ptest::preflight::{self, Configuration, RefusedPart, Requirement};
use crate::service_contract::CovariateContext;
use crate::service_contract::ServiceContract;
use crate::spec::{BaselineSpec, SpecResolver};
use crate::statistics::decision::{DimensionDecisions, compose_overall_verdict};
use crate::statistics::latency::{plan_nondegeneracy, plan_precedence};
use crate::statistics::rules::{EnforcementMode, alpha_from_confidence};
use crate::statistics::types::ConfidenceLevel;
use crate::verdict::{
    AssertionEnforcement, BaselineProvenance, CriterionRow, FunctionalAssessment, SpecProvenance,
    Verdict, VerdictRecord,
};

/// What constitutes acceptable service behaviour.
///
/// Groups the functional success-rate criteria and the latency criteria
/// as peers — both are dimensions of the same question: "is this service
/// good enough?" Provenance fields describe where the criteria came from.
#[derive(Debug, Clone)]
pub struct AssessmentCriteria {
    /// How the sampling plan is resolved.
    pub approach: ThresholdApproach,
    /// Whether this is a verification or smoke test.
    pub intent: TestIntent,
    /// Where declared requirements originate (SLA, SLO, policy); a
    /// baseline-derived criterion is always empirical.
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
    /// Which dimensions bind the test verdict: the run's setting, never the
    /// test's.
    pub enforcement: AssertionEnforcement,
}

/// How to find and interpret empirical reference data.
///
/// The resolved baseline feeds both the functional assessment (the baseline
/// counts each regression cutoff derives from) and the latency assessment
/// (the baseline latencies a precedence threshold derives from).
#[derive(Debug, Clone, Default)]
pub struct BaselineContext {
    /// Filesystem resolver for baseline specs.
    pub spec_resolver: Option<SpecResolver>,
    /// A pre-loaded baseline spec, bypassing the resolver.
    pub pre_resolved_spec: Option<crate::spec::BaselineSpec>,
    /// Covariate context for covariate-aware baseline selection.
    pub covariate_context: Option<CovariateContext>,
}

/// Latency configuration for baseline-derived thresholds carried into the
/// runner; explicit ceilings come from the contract's latency criterion.
#[derive(Debug, Clone, Copy, Default)]
pub struct LatencyConfig {
    /// Confidence of the precedence rank of a baseline-derived threshold.
    pub baseline_confidence: f64,
}

/// The result of a probabilistic test.
///
/// Wraps a [`VerdictRecord`] containing the verdict, statistical analysis,
/// and all supporting evidence — or, for a configuration refused before any
/// sample ran, the configuration errors.
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

    /// Whether the test verdict `V_test` is PASS.
    ///
    /// Composes the dimensions the run enforces; an advisory dimension's
    /// verdict does not affect this result. A refused configuration has not
    /// passed.
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

/// Everything resolved before the first sample: the baseline, the plan, the
/// criteria's baseline tallies and levels, and the latency constraints.
struct Preparation {
    baseline_spec: Option<BaselineSpec>,
    plan: RunPlan,
    criterion_tallies: Vec<CriterionBaselineTally>,
    latency_constraints: Vec<ResolvedLatencyConstraint>,
}

/// Executes a contract-driven probabilistic test: the engine invokes the
/// contract and judges every criterion on every sample, and the verdict
/// decomposes per criterion with a composite over them.
///
/// A configuration with any invalid part — a test larger than a baseline it
/// consumes, or (under verification) a requirement no outcome of the
/// planned size can demonstrate — is refused whole before any sample runs:
/// the result carries a record with every configuration error, no verdict,
/// and the termination reason `CONFIGURATION_REFUSED`.
///
/// # Panics
///
/// Panics if `inputs` is empty, if the sampling plan cannot be resolved (a
/// missing baseline a sized plan or an enforced baseline-derived criterion
/// needs, or a sizing design that cannot be priced), or if a service
/// invocation yields a defect (a transport failure or a caught panic) — a
/// defect aborts the run.
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
    let contract_criteria = contract.criteria();

    let prepared = prepare(
        contract,
        &contract_criteria,
        criteria,
        baseline,
        &service_contract_id,
        &mut warnings,
    );
    let requirements = preflight::requirements(
        &contract_criteria.targets(),
        |name| criterion_confidence(&contract_criteria, name, prepared.plan.confidence).alpha(),
        &prepared.latency_constraints,
    );
    let refused = preflight::refused_parts(
        &Configuration {
            planned_samples: prepared.plan.samples,
            intent: criteria.intent,
            criterion_baselines: &prepared.criterion_tallies,
            baseline_samples: prepared
                .baseline_spec
                .as_ref()
                .map(|s| s.execution.samples_executed),
            latency: &prepared.latency_constraints,
        },
        &requirements,
    );
    if !refused.is_empty() {
        return refused_result(&service_contract_id, criteria, &prepared, &refused);
    }
    record_pre_run_warnings(criteria, &prepared, &requirements, &mut warnings);

    let config = resolve_execution_config(
        config_overrides,
        criteria,
        &prepared,
        validity_floor(&prepared, &requirements),
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
    decide(
        service_contract_id,
        criteria,
        &prepared,
        &contract_criteria,
        &exec_result,
        &config,
        warnings,
    )
}

/// Resolves the baseline, the sampling plan and the latency constraints.
fn prepare<C: ServiceContract>(
    contract: &C,
    contract_criteria: &Criteria<C::Output>,
    criteria: &AssessmentCriteria,
    baseline: BaselineContext,
    service_contract_id: &str,
    warnings: &mut Vec<Warning>,
) -> Preparation
where
    C::Output: 'static,
{
    let baseline_spec = resolve_baseline(baseline, service_contract_id, warnings);
    let criterion_tallies = empirical_criterion_tallies(
        &contract_criteria.targets(),
        baseline_spec.as_ref(),
        criteria.enforcement.functional(),
    );
    let aggregate = baseline_spec.as_ref().map(aggregate_tally);
    let plan = approach::resolve_plan(
        &criteria.approach,
        aggregate.as_ref(),
        &criterion_tallies,
        criteria.enforcement.functional(),
    );
    let latency_constraints = resolve_latency_constraints(
        contract.latency(),
        criteria.latency,
        baseline_spec.as_ref(),
        criteria.enforcement.latency(),
    );
    Preparation {
        baseline_spec,
        plan,
        criterion_tallies,
        latency_constraints,
    }
}

/// The confidence a criterion is decided at: its own, else the test's.
fn criterion_confidence<O: 'static>(
    criteria: &Criteria<O>,
    name: &str,
    test_confidence: ConfidenceLevel,
) -> ConfidenceLevel {
    criteria
        .confidence_of(name)
        .map_or(test_confidence, ConfidenceLevel::new)
}

/// The result of a refused configuration: a record with every configuration
/// error, no verdict, and no sample executed.
fn refused_result(
    service_contract_id: &str,
    criteria: &AssessmentCriteria,
    prepared: &Preparation,
    refused: &[RefusedPart],
) -> ProbabilisticTestResult {
    let details: Vec<String> = refused.iter().map(RefusedPart::describe).collect();
    let execution = ExecutionSummary::new(
        prepared.plan.samples,
        0,
        0,
        0,
        TerminationInfo::new(TerminationReason::ConfigurationRefused)
            .with_detail(details.join("; ")),
        CostSummary::new(std::time::Duration::ZERO, 0, 0),
    );
    let record = VerdictRecord::refused(
        TestIdentity::new(service_contract_id),
        criteria.intent,
        execution,
        refused.iter().map(RefusedPart::code).collect(),
    )
    .confidence_level(prepared.plan.confidence.value())
    .spec_provenance(build_provenance(
        criteria.threshold_origin,
        prepared.baseline_spec.as_ref(),
        criteria.contract_ref.as_deref(),
        None,
    ))
    .build();
    crate::reporting::ConsoleRenderer::new().print_verdict(&record);
    eprintln!(
        "\n{}\n",
        diagnostics::refusal_message(service_contract_id, prepared.plan.samples, refused)
    );
    ProbabilisticTestResult {
        verdict_record: record,
        approach: criteria.approach.clone(),
    }
}

/// Records the warnings a valid configuration still earns before it runs:
/// requirements a smoke run cannot demonstrate, a smoke run's non-evidential
/// verdict, and the latency planning figures (§12.5.3).
fn record_pre_run_warnings(
    criteria: &AssessmentCriteria,
    prepared: &Preparation,
    requirements: &[Requirement],
    warnings: &mut Vec<Warning>,
) {
    if criteria.intent == TestIntent::Smoke {
        for (requirement, check) in
            preflight::infeasible_requirements(prepared.plan.samples, requirements)
        {
            warnings.push(Warning::new(
                "UNDERSIZED",
                diagnostics::infeasibility_message(&requirement.subject, &check, false),
            ));
        }
        if !requirements.is_empty() {
            warnings.push(Warning::new(
                "SMOKE_NORMATIVE",
                "Smoke test against normative requirements — verdict is not evidential",
            ));
        }
    }
    if let Some(spec) = prepared.baseline_spec.as_ref() {
        latency_planning_warnings(
            &prepared.latency_constraints,
            prepared.plan.samples,
            spec,
            warnings,
        );
    }
}

/// The pre-run latency gates on the expected successful count: warnings and
/// planning figures, never a verdict — both are decided after the run on
/// the actual count.
fn latency_planning_warnings(
    constraints: &[ResolvedLatencyConstraint],
    planned: u32,
    spec: &BaselineSpec,
    warnings: &mut Vec<Warning>,
) {
    let rate = spec.statistics.success_rate.observed;
    if rate <= 0.0 {
        return;
    }
    // Every constraint is decided by its rule, enforced or advisory, so every
    // one earns its planning warnings.
    for constraint in constraints {
        let percentile = constraint.percentile().as_fraction();
        if let resolver::ConstraintSource::BaselineDerived {
            baseline_latencies_ms,
        } = constraint.source()
        {
            let baseline = u32::try_from(baseline_latencies_ms.len())
                .expect("baseline latency count fits in u32");
            let alpha = alpha_from_confidence(constraint.confidence());
            let planning = plan_precedence(baseline, planned, rate, percentile, alpha);
            if planning.warning() {
                warnings.push(Warning::new(
                    "LATENCY_SATURATION_EXPECTED",
                    diagnostics::saturation_message(constraint.percentile(), &planning, baseline),
                ));
            }
        }
        // The non-degeneracy gate applies to a baseline-derived constraint;
        // an explicit requirement decides on its within-threshold count
        // instead.
        let planning = plan_nondegeneracy(percentile, planned, rate);
        if constraint.is_baseline_derived() && planning.warning() {
            warnings.push(Warning::new(
                "LATENCY_DEGENERATE_EXPECTED",
                diagnostics::degeneracy_message(constraint.percentile(), &planning),
            ));
        }
    }
}

/// Decides the run: each criterion by its rule, each latency constraint by
/// the rule for its threshold source, and the test verdict by the
/// structural composite of the enforced dimensions, adjusted by the budget
/// and expiration policies.
fn decide<O: 'static>(
    service_contract_id: String,
    criteria: &AssessmentCriteria,
    prepared: &Preparation,
    contract_criteria: &Criteria<O>,
    exec_result: &ContractExecutionResult,
    config: &ExecutionConfig,
    mut warnings: Vec<Warning>,
) -> ProbabilisticTestResult {
    let summary = exec_result.summary();
    let rows = build_criterion_rows(exec_result, contract_criteria, criteria, prepared);
    let latency_dimension = build_latency_dimension(
        &prepared.latency_constraints,
        exec_result.aggregate().successful_latencies(),
        criteria.intent,
        criteria.enforcement.latency(),
    );
    let criterion_verdicts: Vec<(String, Verdict)> = rows
        .iter()
        .map(|row| (row.name().to_owned(), row.verdict()))
        .collect();
    let latency_verdicts = latency_dimension
        .as_ref()
        .map(LatencyDimension::constraint_verdicts)
        .unwrap_or_default();
    let overall = compose_overall_verdict(
        DimensionDecisions {
            decisions: &criterion_verdicts,
            mode: criteria.enforcement.functional(),
        },
        DimensionDecisions {
            decisions: &latency_verdicts,
            mode: criteria.enforcement.latency(),
        },
    );
    let (verdict, expiration_info) = apply_run_policies(
        overall.verdict(),
        summary,
        config,
        prepared.baseline_spec.as_ref(),
        criteria,
        &mut warnings,
    );
    let analysis = rows
        .first()
        .and_then(|row| row.statistical_analysis().cloned());
    let (provenance, baseline_prov, disclosure_entries) =
        build_record_provenance(criteria, prepared, &rows, expiration_info, summary);

    let mut builder = VerdictRecord::builder(
        TestIdentity::new(service_contract_id),
        verdict,
        criteria.intent,
        summary.clone(),
        FunctionalAssessment::new(rows).with_mode(criteria.enforcement.functional()),
    )
    .triggering(overall.triggering().to_vec())
    .confidence_level(prepared.plan.confidence.value())
    .spec_provenance(provenance)
    .environment(disclosure_entries);
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

/// Applies the run-level policies to the test verdict: budget exhaustion and
/// baseline expiration.
fn apply_run_policies(
    verdict: Verdict,
    summary: &ExecutionSummary,
    config: &ExecutionConfig,
    baseline_spec: Option<&BaselineSpec>,
    criteria: &AssessmentCriteria,
    warnings: &mut Vec<Warning>,
) -> (Verdict, Option<ExpirationInfo>) {
    let mut verdict = apply_budget_exhaustion_policy(summary, config, verdict, warnings);
    let expiration_info = baseline_spec.map(crate::spec::expiration::evaluate);
    verdict = apply_expiration_policy(
        expiration_info.as_ref(),
        criteria.fail_on_expired_baseline,
        verdict,
        warnings,
    );
    (verdict, expiration_info)
}

/// Builds the provenance the verdict record carries: the spec provenance,
/// the baseline provenance (when a baseline resolved), and the
/// sizing-transparency facts the report's run-design block renders —
/// computed here, formatted there.
fn build_record_provenance(
    criteria: &AssessmentCriteria,
    prepared: &Preparation,
    rows: &[CriterionRow],
    expiration_info: Option<ExpirationInfo>,
    summary: &ExecutionSummary,
) -> (
    SpecProvenance,
    Option<BaselineProvenance>,
    Vec<(String, String)>,
) {
    let baseline_spec = prepared.baseline_spec.as_ref();
    let provenance = build_provenance(
        criteria.threshold_origin,
        baseline_spec,
        criteria.contract_ref.as_deref(),
        expiration_info,
    );
    let baseline_prov = baseline_spec.map(|spec| build_baseline_provenance(spec, rows));
    let aggregate = baseline_spec.map(aggregate_tally);
    let disclosure_entries = disclosure::sizing_disclosure_entries(
        &criteria.approach,
        &prepared.plan,
        aggregate.as_ref(),
        &prepared.criterion_tallies,
        summary,
    );
    (provenance, baseline_prov, disclosure_entries)
}

/// Derives a baseline provenance record from the resolved spec — filename,
/// timestamp, sample count, observed rate — and the derived threshold: the
/// first baseline-derived criterion's regression cutoff over the test size,
/// or the baseline's stated minimum rate when no criterion derives one.
fn build_baseline_provenance(spec: &BaselineSpec, rows: &[CriterionRow]) -> BaselineProvenance {
    let derived_threshold = rows
        .iter()
        .filter_map(CriterionRow::statistical_analysis)
        .find(|analysis| analysis.threshold_origin() == ThresholdOrigin::Empirical)
        .map_or(spec.requirements.min_pass_rate, |analysis| {
            analysis.threshold()
        });
    BaselineProvenance::new(
        format!("{}.yaml", spec.service_contract_id),
        spec.generated_at.clone(),
        spec.execution.samples_executed,
        spec.statistics.success_rate.observed,
        derived_threshold,
    )
}

/// The fewest samples a run executes before a guaranteed success may stop it
/// early: no fewer than any requirement needs for a count to demonstrate it,
/// and — when the test carries a latency constraint, whose decision depends
/// on how many successful latencies arrive — the whole plan.
fn validity_floor(prepared: &Preparation, requirements: &[Requirement]) -> u32 {
    if !prepared.latency_constraints.is_empty() {
        return prepared.plan.samples;
    }
    requirements
        .iter()
        .map(|r| crate::statistics::compliance::minimum_feasible_samples(r.rate, r.alpha))
        .fold(prepared.plan.validity_floor(), u32::max)
}

/// Synthesises the execution config for a run: an explicit caller override is
/// used as-is, otherwise one is built from the planned sample size and the
/// criteria's budget-exhaustion behaviour. Early termination is wired in only
/// under a meaningful floor — a `0.0` floor (the bare-samples plan) runs
/// every sample and lets each criterion's rule decide the verdict — and
/// only when the functional outcome can end the run (see
/// [`functional_outcome_may_end_run`]).
fn resolve_execution_config(
    config_overrides: Option<&ExecutionConfig>,
    criteria: &AssessmentCriteria,
    prepared: &Preparation,
    validity_floor: u32,
    warmup: u32,
) -> ExecutionConfig {
    let plan = &prepared.plan;
    let mut config = config_overrides.cloned().unwrap_or_else(|| {
        let mut c = ExecutionConfig::new(plan.samples);
        if let Some(behaviour) = criteria.on_budget_exhausted {
            c = c.with_on_budget_exhausted(behaviour);
        }
        c
    });
    if plan.floor > 0.0
        && !criteria.early_termination_disabled
        && functional_outcome_may_end_run(criteria.enforcement, &prepared.latency_constraints)
    {
        config = config
            .min_pass_rate(plan.floor)
            .min_samples_for_validity(validity_floor.max(1));
    }
    config.with_warmup(warmup)
}

/// Whether the functional outcome may end the run early. An advisory
/// functional dimension decides nothing the test verdict depends on, so
/// neither its guaranteed success nor its inevitable failure may cut short
/// the samples a latency constraint is still decided on.
const fn functional_outcome_may_end_run(
    enforcement: AssertionEnforcement,
    latency_constraints: &[ResolvedLatencyConstraint],
) -> bool {
    enforcement.functional().is_enforced() || latency_constraints.is_empty()
}

/// Builds one verdict row per criterion, each decided by its rule.
fn build_criterion_rows<O: 'static>(
    exec_result: &ContractExecutionResult,
    contract_criteria: &Criteria<O>,
    criteria: &AssessmentCriteria,
    prepared: &Preparation,
) -> Vec<CriterionRow> {
    let normative_origin = if criteria.threshold_origin.is_normative() {
        criteria.threshold_origin
    } else {
        ThresholdOrigin::Unspecified
    };
    contract_criteria
        .targets()
        .into_iter()
        .map(|(name, target)| {
            let context = CriterionContext {
                name,
                target,
                confidence: criterion_confidence(contract_criteria, name, prepared.plan.confidence),
                baseline: prepared
                    .criterion_tallies
                    .iter()
                    .find(|tally| tally.criterion_name == name),
                normative_origin,
                design: prepared.plan.design,
            };
            judge::criterion_row(&context, exec_result.criteria_counts())
        })
        .collect()
}

/// Resolves the baseline successes and sample count for an empirical criterion,
/// preferring its own per-criterion measurement and falling back to the
/// whole-contract aggregate when the baseline predates per-criterion capture.
fn criterion_baseline(name: &str, spec: &BaselineSpec) -> CriterionBaselineTally {
    let (successes, trials) = spec
        .statistics
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
        );
    CriterionBaselineTally {
        criterion_name: name.to_owned(),
        successes,
        trials,
    }
}

/// The baseline's whole-contract tally.
fn aggregate_tally(spec: &BaselineSpec) -> CriterionBaselineTally {
    CriterionBaselineTally {
        criterion_name: "contract aggregate".to_owned(),
        successes: spec.statistics.successes,
        trials: spec.execution.samples_executed,
    }
}

/// Resolves each baseline-derived criterion's baseline tally, applying the
/// per-criterion resolution (and whole-contract aggregate fallback) of
/// [`criterion_baseline`].
///
/// With no baseline and the functional dimension advisory, no tally
/// resolves and each baseline-derived criterion is INCONCLUSIVE: a missing
/// baseline does not stop a run in a dimension that binds nothing.
///
/// # Panics
///
/// Panics if the contract carries a baseline-derived criterion, the
/// functional dimension is enforced, and no baseline resolved — an enforced
/// empirical criterion requires one.
fn empirical_criterion_tallies(
    targets: &[(&str, &CriterionTarget)],
    baseline: Option<&BaselineSpec>,
    functional_mode: EnforcementMode,
) -> Vec<CriterionBaselineTally> {
    let empirical = targets
        .iter()
        .filter(|(_, target)| matches!(target, CriterionTarget::EmpiricalRate));
    let Some(spec) = baseline else {
        assert!(
            !functional_mode.is_enforced() || empirical.count() == 0,
            "an empirical criterion requires a baseline"
        );
        return Vec::new();
    };
    empirical
        .map(|(name, _)| criterion_baseline(name, spec))
        .collect()
}

/// Resolves the latency constraints the contract declares: explicit
/// ceilings (at the criterion's confidence) and percentiles asserted against
/// the baseline (at the configured baseline confidence).
///
/// With no baseline latencies and the latency dimension advisory, a
/// percentile asserted against the baseline is resolved without them and
/// judged INCONCLUSIVE: a missing baseline does not stop a run in a
/// dimension that binds nothing.
///
/// # Panics
///
/// Panics if a percentile is asserted against the baseline, the latency
/// dimension is enforced, and no baseline with successful latencies
/// resolved — an enforced baseline-derived constraint requires one.
fn resolve_latency_constraints(
    latency: Option<LatencyCriterion>,
    latency_config: LatencyConfig,
    baseline_spec: Option<&BaselineSpec>,
    latency_mode: EnforcementMode,
) -> Vec<ResolvedLatencyConstraint> {
    let Some(declared) = latency else {
        return Vec::new();
    };
    let baseline_latency = baseline_spec.and_then(|s| s.statistics.latency_distribution.as_ref());
    let constraints = resolver::resolve(
        &declared,
        baseline_latency,
        ConstraintConfidence {
            explicit: declared.decision_confidence(),
            baseline: latency_config.baseline_confidence,
        },
    );
    assert!(
        !latency_mode.is_enforced()
            || !constraints
                .iter()
                .any(ResolvedLatencyConstraint::lacks_baseline_latencies),
        "a latency percentile asserted against the baseline requires a baseline \
         with successful latencies"
    );
    constraints
}

/// Judges the latency constraints after the run on the latencies of the
/// samples that passed every functional criterion, under the run's mode for
/// the latency dimension; `None` when the test carries no latency
/// constraint.
fn build_latency_dimension(
    constraints: &[ResolvedLatencyConstraint],
    successful_latencies: &[std::time::Duration],
    intent: TestIntent,
    mode: EnforcementMode,
) -> Option<LatencyDimension> {
    if constraints.is_empty() {
        return None;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "millisecond latencies fit in f64 mantissa"
    )]
    let latencies_ms: Vec<f64> = successful_latencies
        .iter()
        .map(|d| d.as_millis() as f64)
        .collect();
    Some(LatencyDimension::build(
        &latencies_ms,
        constraints,
        intent,
        mode,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn explicit_p95() -> Vec<ResolvedLatencyConstraint> {
        resolver::resolve(
            &LatencyCriterion::meeting()
                .at_most(crate::latency::Percentile::P95, Duration::from_millis(500)),
            None,
            ConstraintConfidence {
                explicit: 0.95,
                baseline: 0.95,
            },
        )
    }

    #[test]
    fn an_enforced_functional_outcome_may_end_the_run() {
        let enforcement = AssertionEnforcement::ALL_ENFORCED;
        assert!(functional_outcome_may_end_run(enforcement, &explicit_p95()));
        assert!(functional_outcome_may_end_run(enforcement, &[]));
    }

    #[test]
    fn an_advisory_functional_outcome_never_cuts_latency_samples_short() {
        let enforcement = AssertionEnforcement::parse("functional");
        assert!(!functional_outcome_may_end_run(
            enforcement,
            &explicit_p95()
        ));
        assert!(functional_outcome_may_end_run(enforcement, &[]));
    }

    #[test]
    fn without_a_baseline_an_advisory_functional_dimension_resolves_no_tally() {
        let target = CriterionTarget::EmpiricalRate;
        let targets = [("c", &target)];
        let tallies = empirical_criterion_tallies(&targets, None, EnforcementMode::Advisory);
        assert_eq!(tallies.len(), 0);
    }

    #[test]
    #[should_panic(expected = "an empirical criterion requires a baseline")]
    fn without_a_baseline_an_enforced_empirical_criterion_stops_the_run() {
        let target = CriterionTarget::EmpiricalRate;
        let targets = [("c", &target)];
        let _ = empirical_criterion_tallies(&targets, None, EnforcementMode::Enforced);
    }

    #[test]
    fn without_a_baseline_a_normative_criterion_needs_none() {
        let target = CriterionTarget::NormativeRate(0.9);
        let targets = [("c", &target)];
        let tallies = empirical_criterion_tallies(&targets, None, EnforcementMode::Enforced);
        assert_eq!(tallies.len(), 0);
    }

    fn asserted_against_baseline() -> LatencyCriterion {
        LatencyCriterion::empirical().against_baseline(crate::latency::Percentile::P95)
    }

    const LATENCY_CONFIG: LatencyConfig = LatencyConfig {
        baseline_confidence: 0.95,
    };

    #[test]
    fn without_baseline_latencies_an_advisory_latency_dimension_still_resolves() {
        let constraints = resolve_latency_constraints(
            Some(asserted_against_baseline()),
            LATENCY_CONFIG,
            None,
            EnforcementMode::Advisory,
        );
        assert_eq!(constraints.len(), 1);
        assert!(constraints[0].lacks_baseline_latencies());
    }

    #[test]
    #[should_panic(expected = "requires a baseline with successful latencies")]
    fn without_baseline_latencies_an_enforced_baseline_constraint_stops_the_run() {
        let _ = resolve_latency_constraints(
            Some(asserted_against_baseline()),
            LATENCY_CONFIG,
            None,
            EnforcementMode::Enforced,
        );
    }
}
