//! Probabilistic test execution and verdict production.

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::ExecutionEngine;
use crate::latency::{
    LatencyDimension, LatencyEnforcementMode, LatencyThresholds, enforcement, resolver,
};
use crate::model::{TestIdentity, TestIntent, ThresholdOrigin, TrialOutcome, Warning};
use crate::ptest::approach;
use crate::ptest::builder::ThresholdApproach;
use crate::ptest::diagnostics;
use crate::spec::SpecResolver;
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
#[allow(clippy::too_many_lines)]
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

    let baseline_spec = baseline.pre_resolved_spec.or_else(|| {
        baseline.spec_resolver.as_ref().and_then(|resolver| {
            crate::ptest::baseline::resolve(
                resolver,
                use_case_id,
                baseline.covariate_context.as_ref(),
                &mut warnings,
            )
        })
    });

    let (samples, derived_threshold) = approach::resolve_threshold(
        &criteria.approach,
        baseline_spec.as_ref().map(|s| &s.statistics),
        baseline_spec.as_ref().map(|s| &s.execution),
    );

    let resolved_confidence = approach::resolved_confidence(&criteria.approach);
    let feas =
        feasibility::feasibility_check(samples, derived_threshold.value(), resolved_confidence);

    if !feas.feasible() {
        match criteria.intent {
            TestIntent::Verification => {
                panic!(
                    "\n\n{}\n",
                    diagnostics::infeasibility_message(use_case_id, &feas, false),
                );
            }
            TestIntent::Smoke => {
                warnings.push(Warning::new(
                    "UNDERSIZED",
                    diagnostics::infeasibility_message(use_case_id, &feas, false),
                ));
            }
        }
    }

    let config = config_overrides
        .cloned()
        .unwrap_or_else(|| ExecutionConfig::new(samples));

    let token_recorder = TokenRecorder::new();
    let exec_result = ExecutionEngine::run(&config, inputs, &token_recorder, trial);

    let summary = exec_result.summary();
    let aggregate = exec_result.aggregate();

    let stats_verdict = evaluator::evaluate(
        summary.successes(),
        summary.samples_executed(),
        &derived_threshold,
    );

    let verdict = if stats_verdict.passed() {
        Verdict::Pass
    } else {
        Verdict::Fail
    };

    if criteria.intent == TestIntent::Smoke && criteria.threshold_origin.is_normative() {
        warnings.push(Warning::new(
            "SMOKE_NORMATIVE",
            "Smoke test against normative threshold — verdict is not evidential",
        ));
    }

    let analysis = build_analysis(summary, &derived_threshold, criteria.threshold_origin);
    let provenance = build_provenance(
        criteria.threshold_origin,
        baseline_spec.as_ref(),
        criteria.contract_ref.as_deref(),
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

    let baseline_prov = baseline_spec.as_ref().map(|spec| {
        BaselineProvenance::new(
            format!("{}.yaml", spec.use_case_id),
            spec.generated_at.clone(),
            spec.execution.samples_executed,
            spec.statistics.success_rate.observed,
            spec.requirements.min_pass_rate,
        )
    });

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

    #[allow(clippy::cast_precision_loss)]
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

    let (se, ci_lower, ci_upper) = if summary.samples_executed() > 0 {
        let se = proportion::standard_error(summary.successes(), summary.samples_executed());
        let estimate = proportion::estimate(
            summary.successes(),
            summary.samples_executed(),
            derived_threshold.context().confidence(),
        );
        (se, estimate.lower_bound(), estimate.upper_bound())
    } else {
        (0.0, 0.0, 0.0)
    };

    let mut analysis = StatisticalAnalysis::new(
        confidence_level,
        se,
        ci_lower,
        ci_upper,
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
) -> SpecProvenance {
    let mut provenance = SpecProvenance::new(threshold_origin);
    if let Some(spec) = baseline_spec {
        provenance = provenance.with_spec_filename(format!("{}.yaml", spec.use_case_id));
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
        let uc = SpecTestUc;
        let inputs = vec!["input".to_string()];
        let measure_result =
            crate::experiment::MeasureExperiment::new(&uc, 200, &inputs, always_succeeds)
                .with_spec_resolver(crate::spec::SpecResolver::with_dir(dir.path()))
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
        let uc = ConfTestUc;
        let inputs = vec!["input".to_string()];
        crate::experiment::MeasureExperiment::new(&uc, 200, &inputs, always_succeeds)
            .with_spec_resolver(crate::spec::SpecResolver::with_dir(dir.path()))
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
    #[should_panic(expected = "threshold approach must be set")]
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

        crate::experiment::MeasureExperiment::new(&uc, 100, &inputs, always_succeeds)
            .with_spec_resolver(crate::spec::SpecResolver::with_dir(dir.path()))
            .covariates(vec!["model".to_string()], profile)
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
            .use_case(&uc)
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

        let uc = SimpleUc;
        let inputs = vec!["input".to_string()];
        crate::experiment::MeasureExperiment::new(&uc, 100, &inputs, always_succeeds)
            .with_spec_resolver(crate::spec::SpecResolver::with_dir(dir.path()))
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
}
