//! Probabilistic test execution and verdict production.

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::ExecutionEngine;
use crate::model::{TestIdentity, TestIntent, ThresholdOrigin, TrialOutcome, Warning};
use crate::ptest::builder::ThresholdApproach;
use crate::spec::SpecResolver;
use crate::statistics::types::ConfidenceLevel;
use crate::statistics::{defaults, evaluator, feasibility, proportion, sample_size, threshold};
use crate::verdict::{
    FunctionalDimension, SpecProvenance, StatisticalAnalysis, Verdict, VerdictRecord,
};

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
}

impl ProbabilisticTestResult {
    /// The full verdict record.
    #[must_use]
    pub const fn verdict_record(&self) -> &VerdictRecord {
        &self.verdict_record
    }

    /// Whether the test passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.verdict_record.verdict() == Verdict::Pass
    }
}

/// Executes a probabilistic test and produces a verdict.
#[allow(clippy::too_many_arguments)]
pub fn execute<F>(
    use_case_id: &str,
    inputs: &[String],
    trial: F,
    approach: &ThresholdApproach,
    intent: TestIntent,
    threshold_origin: ThresholdOrigin,
    contract_ref: Option<&str>,
    spec_resolver: Option<&SpecResolver>,
    pre_resolved_spec: Option<crate::spec::BaselineSpec>,
    config_overrides: Option<&ExecutionConfig>,
) -> ProbabilisticTestResult
where
    F: FnMut(&str) -> TrialOutcome,
{
    let mut warnings: Vec<Warning> = Vec::new();

    // Use pre-resolved spec if provided, otherwise resolve from filesystem
    let baseline_spec = pre_resolved_spec
        .or_else(|| spec_resolver.and_then(|resolver| resolver.resolve(use_case_id).ok()));

    // Determine samples and threshold based on the approach
    let (samples, derived_threshold) = resolve_threshold(
        approach,
        baseline_spec.as_ref().map(|s| &s.statistics),
        baseline_spec.as_ref().map(|s| &s.execution),
    );

    // Pre-flight feasibility check for normative + verification
    if intent == TestIntent::Verification && threshold_origin.is_normative() {
        let target = derived_threshold.value();
        let alpha = defaults::DEFAULT_ALPHA;
        if feasibility::is_undersized(samples, target, alpha) {
            warnings.push(Warning::new(
                "UNDERSIZED",
                format!(
                    "Sample size {samples} is insufficient for verification-grade \
                     evidence at threshold {target:.4}"
                ),
            ));
        }
    }

    // Build execution config
    let config = config_overrides
        .cloned()
        .unwrap_or_else(|| ExecutionConfig::new(samples));

    // Run trials
    let token_recorder = TokenRecorder::new();
    let exec_result = ExecutionEngine::run(&config, inputs, &token_recorder, trial);

    let summary = exec_result.summary();
    let aggregate = exec_result.aggregate();

    // Evaluate verdict using the statistics evaluator
    let stats_verdict = evaluator::evaluate(
        summary.successes(),
        summary.samples_executed(),
        &derived_threshold,
    );

    // Map to framework verdict
    let verdict = if stats_verdict.passed() {
        Verdict::Pass
    } else {
        Verdict::Fail
    };

    // Check for smoke intent caveat
    if intent == TestIntent::Smoke && threshold_origin.is_normative() {
        warnings.push(Warning::new(
            "SMOKE_NORMATIVE",
            "Smoke test against normative threshold — verdict is not evidential",
        ));
    }

    // Build verdict record components
    let analysis = build_analysis(summary, &derived_threshold, threshold_origin);
    let provenance = build_provenance(threshold_origin, baseline_spec.as_ref(), contract_ref);
    let functional = FunctionalDimension::new(
        summary.successes(),
        summary.failures(),
        aggregate.failure_distribution().to_vec(),
    );

    // Assemble the verdict record
    let identity = TestIdentity::new(use_case_id);
    let mut builder =
        VerdictRecord::builder(identity, verdict, intent, summary.clone(), functional)
            .statistical_analysis(analysis)
            .spec_provenance(provenance);
    for w in warnings {
        builder = builder.warning(w);
    }

    ProbabilisticTestResult {
        verdict_record: builder.build(),
    }
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

/// Resolves the sample count and derived threshold from the approach.
fn resolve_threshold(
    approach: &ThresholdApproach,
    stats: Option<&crate::spec::baseline::StatisticsBlock>,
    execution: Option<&crate::spec::baseline::ExecutionBlock>,
) -> (u32, crate::statistics::types::DerivedThreshold) {
    match approach {
        ThresholdApproach::SampleSizeFirst {
            samples,
            confidence,
        } => {
            let conf = ConfidenceLevel::new(*confidence);
            // Need baseline data from spec
            let (baseline_successes, baseline_samples) = extract_baseline(stats, execution);
            let derived = threshold::derive_sample_size_first(
                baseline_successes,
                baseline_samples,
                *samples,
                conf,
            );
            (*samples, derived)
        }

        ThresholdApproach::ConfidenceFirst {
            confidence,
            min_detectable_effect,
            power,
        } => {
            let conf = ConfidenceLevel::new(*confidence);
            let (baseline_successes, baseline_samples) = extract_baseline(stats, execution);
            let baseline_rate = f64::from(baseline_successes) / f64::from(baseline_samples);

            // Compute required sample size
            let requirement = sample_size::calculate_for_power(
                baseline_rate,
                *min_detectable_effect,
                conf,
                *power,
            );

            let samples = requirement.required_samples();
            let derived = threshold::derive_sample_size_first(
                baseline_successes,
                baseline_samples,
                samples,
                conf,
            );
            (samples, derived)
        }

        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        } => {
            // If we have baseline data, use it for threshold-first derivation
            if let (Some(s), Some(e)) = (stats, execution) {
                let baseline_successes = s.successes;
                let baseline_samples = e.samples_executed;
                let derived = threshold::derive_threshold_first(
                    baseline_successes,
                    baseline_samples,
                    *samples,
                    *min_pass_rate,
                );
                (*samples, derived)
            } else {
                // No baseline — use explicit threshold with default confidence.
                // We use the threshold as the synthetic baseline rate and
                // set baseline_samples = test_samples as a placeholder,
                // since there is no real baseline to reference.
                let conf = ConfidenceLevel::new(defaults::DEFAULT_CONFIDENCE);
                let context = crate::statistics::types::DerivationContext::new(
                    *min_pass_rate,
                    *samples,
                    *samples,
                    conf,
                );
                let derived = crate::statistics::types::DerivedThreshold::new(
                    *min_pass_rate,
                    crate::statistics::types::OperationalApproach::ThresholdFirst,
                    context,
                    false,
                );
                (*samples, derived)
            }
        }
    }
}

/// Extracts baseline successes and sample count from spec blocks.
///
/// # Panics
///
/// Panics if no baseline data is available (spec is required for
/// sample-size-first and confidence-first approaches).
const fn extract_baseline(
    stats: Option<&crate::spec::baseline::StatisticsBlock>,
    execution: Option<&crate::spec::baseline::ExecutionBlock>,
) -> (u32, u32) {
    let stats = stats.expect("baseline spec required for this threshold approach");
    let execution = execution.expect("baseline spec required for this threshold approach");
    (stats.successes, execution.samples_executed)
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
        // 8 out of 10 inputs are "ok", 2 are "fail" — cycling 50 samples gives 80% pass rate
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
                samples: 50,
                min_pass_rate: 0.95,
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
                samples: 20,
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
        let inputs = vec!["input".to_string()];
        let measure_result =
            crate::experiment::MeasureExperiment::new("spec-test", 200, &inputs, always_succeeds)
                .with_spec_resolver(crate::spec::SpecResolver::with_dir(dir.path()))
                .run();

        assert!(measure_result.spec_path().is_some());

        // Now run a probabilistic test using the spec
        let result = ProbabilisticTestBuilder::new("spec-test", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 50,
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

        let inputs = vec!["input".to_string()];
        crate::experiment::MeasureExperiment::new("conf-test", 200, &inputs, always_succeeds)
            .with_spec_resolver(crate::spec::SpecResolver::with_dir(dir.path()))
            .run();

        let resolver = crate::spec::SpecResolver::with_dir(dir.path());
        let result = ProbabilisticTestBuilder::new("conf-test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 0.95,
                min_detectable_effect: 0.05,
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
}
