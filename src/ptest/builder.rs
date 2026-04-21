//! Builder for configuring and launching a probabilistic test.

use std::time::Duration;

use crate::controls::ExecutionConfig;
use crate::latency::{LatencyEnforcementMode, LatencyThresholds, Percentile};
use crate::model::{BudgetExhaustedBehavior, TestIntent, ThresholdOrigin, TrialOutcome};
use crate::ptest::runner::{
    self, AssessmentCriteria, BaselineContext, LatencyConfig, ProbabilisticTestResult,
};
use crate::ptest::validation::{self, MacroConfig};
use crate::spec::{BaselineSpec, SpecResolver};
use crate::usecase::{CovariateContext, UseCase};

/// Configures the threshold derivation approach.
///
/// Exactly one approach must be specified. The framework derives the
/// remaining parameters from the baseline spec and the chosen approach.
#[derive(Debug, Clone)]
pub enum ThresholdApproach {
    /// Fix samples and confidence; derive threshold from baseline spec.
    ///
    /// The threshold is the Wilson lower bound at the given confidence level.
    SampleSizeFirst {
        /// Number of test samples.
        samples: u32,
        /// Confidence level for threshold derivation.
        confidence: f64,
    },

    /// Fix confidence, effect size, and power; derive required sample count.
    ///
    /// The framework computes the minimum sample size needed to detect
    /// a degradation of `min_detectable_effect` with the given power.
    ConfidenceFirst {
        /// Required confidence level.
        confidence: f64,
        /// Smallest degradation worth detecting (absolute drop in pass rate).
        min_detectable_effect: f64,
        /// Probability of detecting a real degradation.
        power: f64,
    },

    /// Fix samples and an explicit threshold; framework derives implied confidence.
    ThresholdFirst {
        /// Number of test samples.
        samples: u32,
        /// Explicit minimum pass rate.
        min_pass_rate: f64,
    },
}

/// Builder for a probabilistic test.
///
/// Configures the test parameters and launches execution. The builder
/// requires a threshold approach and a trial closure.
///
/// # Examples
///
/// Threshold-first: "I know the pass rate must be at least 90%."
///
/// ```
/// use feotest::ptest::ProbabilisticTestBuilder;
/// use feotest::ptest::builder::ThresholdApproach;
/// use feotest::model::TrialOutcome;
/// use feotest::verdict::Verdict;
/// use std::time::Duration;
///
/// let inputs = vec!["request".to_string()];
/// let result = ProbabilisticTestBuilder::new(
///     "my-service",
///     &inputs,
///     |_input| TrialOutcome::success(Duration::from_millis(1)),
/// )
/// .approach(ThresholdApproach::ThresholdFirst {
///     samples: 50,
///     min_pass_rate: 0.90,
/// })
/// .run();
///
/// assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
/// ```
///
/// With intent and threshold origin for SLA verification:
///
/// ```
/// use feotest::ptest::ProbabilisticTestBuilder;
/// use feotest::ptest::builder::ThresholdApproach;
/// use feotest::model::{TestIntent, ThresholdOrigin, TrialOutcome};
/// use std::time::Duration;
///
/// let inputs = vec!["request".to_string()];
/// let result = ProbabilisticTestBuilder::new("payment-gateway", &inputs,
///     |_input| TrialOutcome::success(Duration::from_millis(5)),
/// )
/// .approach(ThresholdApproach::ThresholdFirst {
///     samples: 100,
///     min_pass_rate: 0.99,
/// })
/// .intent(TestIntent::Smoke)
/// .threshold_origin(ThresholdOrigin::Sla)
/// .contract_ref("Payment SLA v2.3 §4.1")
/// .run();
///
/// let prov = result.verdict_record().spec_provenance().unwrap();
/// assert_eq!(prov.contract_ref(), Some("Payment SLA v2.3 §4.1"));
/// ```
pub struct ProbabilisticTestBuilder<'a, F> {
    use_case_id: String,
    inputs: &'a [String],
    trial: F,
    approach: Option<ThresholdApproach>,
    intent: TestIntent,
    threshold_origin: ThresholdOrigin,
    contract_ref: Option<String>,
    spec_resolver: Option<SpecResolver>,
    baseline_spec: Option<BaselineSpec>,
    config_overrides: Option<ExecutionConfig>,
    transparent_stats: bool,
    covariate_context: Option<CovariateContext>,
    latency_thresholds: LatencyThresholds,
    baseline_latency_mode: Option<LatencyEnforcementMode>,
    baseline_latency_confidence: Option<f64>,
    fail_on_expired_baseline: bool,
    on_budget_exhausted: Option<BudgetExhaustedBehavior>,
}

impl<'a, F> ProbabilisticTestBuilder<'a, F>
where
    F: FnMut(&str) -> TrialOutcome,
{
    /// Creates a new probabilistic test builder.
    ///
    /// # Panics
    ///
    /// Panics if `inputs` is empty.
    pub fn new(use_case_id: impl Into<String>, inputs: &'a [String], trial: F) -> Self {
        assert!(!inputs.is_empty(), "inputs must not be empty");
        Self {
            use_case_id: use_case_id.into(),
            inputs,
            trial,
            approach: None,
            intent: TestIntent::Verification,
            threshold_origin: ThresholdOrigin::Unspecified,
            contract_ref: None,
            spec_resolver: None,
            baseline_spec: None,
            config_overrides: None,
            transparent_stats: false,
            covariate_context: None,
            latency_thresholds: LatencyThresholds::new(),
            baseline_latency_mode: None,
            baseline_latency_confidence: None,
            fail_on_expired_baseline: false,
            on_budget_exhausted: None,
        }
    }

    /// Sets the threshold derivation approach.
    #[must_use]
    pub const fn approach(mut self, approach: ThresholdApproach) -> Self {
        self.approach = Some(approach);
        self
    }

    /// Sets the test intent.
    #[must_use]
    pub const fn intent(mut self, intent: TestIntent) -> Self {
        self.intent = intent;
        self
    }

    /// Sets the threshold origin.
    #[must_use]
    pub const fn threshold_origin(mut self, origin: ThresholdOrigin) -> Self {
        self.threshold_origin = origin;
        self
    }

    /// Sets a human-readable contract reference.
    #[must_use]
    pub fn contract_ref(mut self, reference: impl Into<String>) -> Self {
        self.contract_ref = Some(reference.into());
        self
    }

    /// Sets the spec resolver for baseline-driven threshold derivation.
    #[must_use]
    pub fn spec_resolver(mut self, resolver: SpecResolver) -> Self {
        self.spec_resolver = Some(resolver);
        self
    }

    /// Sets a pre-resolved baseline spec directly.
    ///
    /// Use this when the spec has already been loaded (e.g., by the
    /// `#[probabilistic_test]` macro) instead of going through the
    /// resolver.
    #[must_use]
    pub fn baseline_spec(mut self, spec: BaselineSpec) -> Self {
        self.baseline_spec = Some(spec);
        self
    }

    /// Overrides execution configuration (warmup, budgets, pacing).
    #[must_use]
    pub const fn execution_config(mut self, config: ExecutionConfig) -> Self {
        self.config_overrides = Some(config);
        self
    }

    /// Sets the behaviour when a budget is exhausted.
    ///
    /// If a full [`ExecutionConfig`] is also supplied via
    /// [`Self::execution_config`], that config's own setting wins —
    /// this setter only has effect when the runner synthesises a default
    /// config.
    #[must_use]
    pub const fn on_budget_exhausted(mut self, behaviour: BudgetExhaustedBehavior) -> Self {
        self.on_budget_exhausted = Some(behaviour);
        self
    }

    /// Enables transparent statistics in the verdict output.
    #[must_use]
    pub const fn transparent_stats(mut self, enabled: bool) -> Self {
        self.transparent_stats = enabled;
        self
    }

    /// Declares an explicit p50 latency threshold. Strictly enforced.
    #[must_use]
    pub fn latency_p50(mut self, value: Duration) -> Self {
        self.latency_thresholds = self.latency_thresholds.with(Percentile::P50, value);
        self
    }

    /// Declares an explicit p90 latency threshold. Strictly enforced.
    #[must_use]
    pub fn latency_p90(mut self, value: Duration) -> Self {
        self.latency_thresholds = self.latency_thresholds.with(Percentile::P90, value);
        self
    }

    /// Declares an explicit p95 latency threshold. Strictly enforced.
    #[must_use]
    pub fn latency_p95(mut self, value: Duration) -> Self {
        self.latency_thresholds = self.latency_thresholds.with(Percentile::P95, value);
        self
    }

    /// Declares an explicit p99 latency threshold. Strictly enforced.
    #[must_use]
    pub fn latency_p99(mut self, value: Duration) -> Self {
        self.latency_thresholds = self.latency_thresholds.with(Percentile::P99, value);
        self
    }

    /// Controls whether baseline-derived latency thresholds fail the verdict
    /// on violation (`true` → `Strict`) or warn only (`false` → `Advisory`).
    ///
    /// When unset, the `FEOTEST_LATENCY_ENFORCE` env var is consulted, then
    /// `Advisory` is the default. Explicit thresholds are always strict.
    #[must_use]
    pub const fn enforce_baseline_latency(mut self, strict: bool) -> Self {
        self.baseline_latency_mode = Some(if strict {
            LatencyEnforcementMode::Strict
        } else {
            LatencyEnforcementMode::Advisory
        });
        self
    }

    /// Overrides the confidence level used when deriving a latency threshold
    /// from the baseline (default `0.95`).
    #[must_use]
    pub const fn baseline_latency_confidence(mut self, confidence: f64) -> Self {
        self.baseline_latency_confidence = Some(confidence);
        self
    }

    /// Escalates expired baselines from warning to test failure.
    ///
    /// By default, loading an expired baseline emits a warning in the
    /// verdict report but does not affect the pass/fail outcome. Setting
    /// this to `true` causes the runner to produce
    /// [`crate::verdict::Verdict::Fail`] whenever the resolved baseline
    /// has an [`crate::model::ExpirationStatus::Expired`] status.
    ///
    /// Has no effect on baselines without an expiration policy or on
    /// those still within their validity window.
    #[must_use]
    pub const fn fail_on_expired_baseline(mut self, fail: bool) -> Self {
        self.fail_on_expired_baseline = fail;
        self
    }

    /// Sets covariate context from a use case for baseline selection.
    ///
    /// When set, the resolver uses covariate-aware selection to find
    /// the best-matching baseline rather than returning the first match.
    /// If the use case declares no covariates, this is a no-op.
    #[must_use]
    pub fn use_case(mut self, use_case: &dyn UseCase) -> Self {
        self.covariate_context = CovariateContext::from_use_case(use_case);
        self
    }

    /// Runs the probabilistic test and returns the result.
    ///
    /// # Panics
    ///
    /// Panics if no threshold approach has been set, or if any parameter on
    /// the selected [`ThresholdApproach`] is outside its valid range. See
    /// [`validate_approach_bounds`] for the per-parameter constraints.
    pub fn run(self) -> ProbabilisticTestResult {
        let approach = self
            .approach
            .expect("threshold approach must be set before running");

        validate_approach_bounds(&approach);

        // Coherence validation — same rules as the macro path
        let config = macro_config_from_approach(
            &self.use_case_id,
            &approach,
            self.threshold_origin,
            self.baseline_spec.is_some() || self.spec_resolver.is_some(),
        );
        validation::validate(&config);

        let transparent_stats = self.transparent_stats;
        let criteria = AssessmentCriteria {
            approach,
            intent: self.intent,
            threshold_origin: self.threshold_origin,
            contract_ref: self.contract_ref,
            latency: LatencyConfig {
                thresholds: self.latency_thresholds,
                baseline_mode: self.baseline_latency_mode,
                baseline_confidence: self
                    .baseline_latency_confidence
                    .unwrap_or(crate::latency::DEFAULT_BASELINE_CONFIDENCE),
            },
            fail_on_expired_baseline: self.fail_on_expired_baseline,
            on_budget_exhausted: self.on_budget_exhausted,
        };
        let baseline = BaselineContext {
            spec_resolver: self.spec_resolver,
            pre_resolved_spec: self.baseline_spec,
            covariate_context: self.covariate_context,
        };

        let result = runner::execute(
            &self.use_case_id,
            self.inputs,
            self.trial,
            &criteria,
            baseline,
            self.config_overrides.as_ref(),
        );

        // Always print the console verdict
        let renderer = crate::reporting::ConsoleRenderer::new();
        renderer.print_verdict(result.verdict_record());

        if transparent_stats {
            let mut buf = String::new();
            crate::reporting::transparent::render(
                result.verdict_record(),
                result.approach(),
                &mut buf,
            )
            .expect("formatting should not fail");
            eprint!("{buf}");
        }

        result
    }
}

/// Constructs a `MacroConfig` from a `ThresholdApproach` for coherence validation.
pub(crate) fn macro_config_from_approach(
    test_name: &str,
    approach: &ThresholdApproach,
    threshold_origin: ThresholdOrigin,
    has_baseline: bool,
) -> MacroConfig {
    match approach {
        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        } => MacroConfig {
            test_name: test_name.to_string(),
            samples: Some(*samples),
            threshold: Some(*min_pass_rate),
            confidence: None,
            min_detectable_effect: None,
            power: None,
            threshold_origin,
            has_baseline,
            baseline_rate: None,
        },
        ThresholdApproach::SampleSizeFirst {
            samples,
            confidence,
        } => MacroConfig {
            test_name: test_name.to_string(),
            samples: Some(*samples),
            threshold: None,
            confidence: Some(*confidence),
            min_detectable_effect: None,
            power: None,
            threshold_origin,
            has_baseline,
            baseline_rate: None,
        },
        ThresholdApproach::ConfidenceFirst {
            confidence,
            min_detectable_effect,
            power,
        } => MacroConfig {
            test_name: test_name.to_string(),
            samples: None,
            threshold: None,
            confidence: Some(*confidence),
            min_detectable_effect: Some(*min_detectable_effect),
            power: Some(*power),
            threshold_origin,
            has_baseline,
            baseline_rate: None,
        },
    }
}

/// Validates parameter bounds on a threshold approach before execution.
///
/// # Panics
///
/// Panics if any parameter on the supplied approach is outside its valid
/// range:
///
/// - `samples` must be `> 0`.
/// - `min_pass_rate` must be in `[0, 1]`.
/// - `confidence`, `min_detectable_effect`, and `power` must be in `(0, 1)`.
pub(crate) fn validate_approach_bounds(approach: &ThresholdApproach) {
    match approach {
        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        } => {
            assert_samples_positive(*samples);
            assert!(
                (0.0..=1.0).contains(min_pass_rate),
                "min_pass_rate must be in [0, 1], got {min_pass_rate}"
            );
        }
        ThresholdApproach::SampleSizeFirst {
            samples,
            confidence,
        } => {
            assert_samples_positive(*samples);
            assert_in_open_unit_interval("confidence", *confidence);
        }
        ThresholdApproach::ConfidenceFirst {
            confidence,
            min_detectable_effect,
            power,
        } => {
            assert_in_open_unit_interval("confidence", *confidence);
            assert_in_open_unit_interval("min_detectable_effect", *min_detectable_effect);
            assert_in_open_unit_interval("power", *power);
        }
    }
}

fn assert_samples_positive(samples: u32) {
    assert!(samples > 0, "samples must be > 0, got {samples}");
}

fn assert_in_open_unit_interval(parameter: &str, value: f64) {
    assert!(
        value > 0.0 && value < 1.0,
        "{parameter} must be in (0, 1), got {value}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TestIntent, ThresholdOrigin, TrialOutcome};
    use crate::verdict::Verdict;
    use std::time::Duration;

    fn always_succeeds(_input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    // --- Builder construction and configuration ---

    #[test]
    #[should_panic(expected = "threshold approach must be set")]
    fn panics_without_approach() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds).run();
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn panics_on_empty_inputs() {
        let inputs: Vec<String> = vec![];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.90,
            })
            .run();
    }

    #[test]
    fn threshold_first_produces_pass() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.90,
            })
            .run();
        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
    }

    #[test]
    fn sample_size_first_produces_pass() {
        let dir = tempfile::tempdir().unwrap();
        struct Uc;
        impl crate::usecase::UseCase for Uc {
            fn id(&self) -> &str {
                "ssf-pass"
            }
        }
        let uc = Uc;
        let inputs = vec!["input".to_string()];
        crate::experiment::MeasureExperiment::new(&uc, 200, &inputs, always_succeeds)
            .with_spec_resolver(crate::spec::SpecResolver::with_dir(dir.path()))
            .run();

        let resolver = crate::spec::SpecResolver::with_dir(dir.path());
        let result = ProbabilisticTestBuilder::new("ssf-pass", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 200,
                confidence: 0.95,
            })
            .spec_resolver(resolver)
            .threshold_origin(ThresholdOrigin::Empirical)
            .run();
        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
    }

    // --- Intent and provenance propagation ---

    #[test]
    fn intent_propagates_to_verdict() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.90,
            })
            .intent(TestIntent::Smoke)
            .run();
        assert_eq!(result.verdict_record().intent(), TestIntent::Smoke);
    }

    #[test]
    fn threshold_origin_propagates() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.90,
            })
            .threshold_origin(ThresholdOrigin::Sla)
            .run();
        let prov = result.verdict_record().spec_provenance().unwrap();
        assert_eq!(prov.threshold_origin(), ThresholdOrigin::Sla);
    }

    #[test]
    fn contract_ref_propagates() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.90,
            })
            .threshold_origin(ThresholdOrigin::Sla)
            .contract_ref("SLA v2")
            .run();
        let prov = result.verdict_record().spec_provenance().unwrap();
        assert_eq!(prov.contract_ref(), Some("SLA v2"));
    }

    #[test]
    fn approach_stored_on_result() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.90,
            })
            .run();
        assert!(matches!(
            result.approach(),
            ThresholdApproach::ThresholdFirst { .. }
        ));
    }

    // --- Validation: parameter bounds ---

    #[test]
    #[should_panic(expected = "min_pass_rate must be in [0, 1], got 1.5")]
    fn panics_on_min_pass_rate_above_one() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 10,
                min_pass_rate: 1.5,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "min_pass_rate must be in [0, 1], got -0.1")]
    fn panics_on_min_pass_rate_negative() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 10,
                min_pass_rate: -0.1,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "samples must be > 0, got 0")]
    fn panics_on_zero_samples_threshold_first() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 0,
                min_pass_rate: 0.90,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "samples must be > 0, got 0")]
    fn panics_on_zero_samples_sample_size_first() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 0,
                confidence: 0.95,
            })
            .run();
    }

    #[test]
    fn accepts_min_pass_rate_zero() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 0.0,
            })
            .run();
        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
    }

    #[test]
    fn accepts_min_pass_rate_one() {
        // min_pass_rate = 1.0 is a valid boundary but is inherently
        // infeasible at any finite sample count. Smoke intent surfaces
        // infeasibility as a warning rather than a panic, letting the
        // test run to completion with always-successful trials.
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 50,
                min_pass_rate: 1.0,
            })
            .intent(TestIntent::Smoke)
            .run();
        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
    }

    #[test]
    #[should_panic(expected = "confidence must be in (0, 1), got 0")]
    fn panics_on_sample_size_first_confidence_zero() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 50,
                confidence: 0.0,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "confidence must be in (0, 1), got 1")]
    fn panics_on_sample_size_first_confidence_one() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 50,
                confidence: 1.0,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "confidence must be in (0, 1), got 1.2")]
    fn panics_on_sample_size_first_confidence_above_one() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 50,
                confidence: 1.2,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "confidence must be in (0, 1), got 0")]
    fn panics_on_confidence_first_confidence_zero() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 0.0,
                min_detectable_effect: 0.05,
                power: 0.80,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "confidence must be in (0, 1), got 1")]
    fn panics_on_confidence_first_confidence_one() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 1.0,
                min_detectable_effect: 0.05,
                power: 0.80,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "confidence must be in (0, 1), got -0.01")]
    fn panics_on_confidence_first_confidence_negative() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: -0.01,
                min_detectable_effect: 0.05,
                power: 0.80,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "power must be in (0, 1), got 0")]
    fn panics_on_confidence_first_power_zero() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 0.95,
                min_detectable_effect: 0.05,
                power: 0.0,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "power must be in (0, 1), got 1")]
    fn panics_on_confidence_first_power_one() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 0.95,
                min_detectable_effect: 0.05,
                power: 1.0,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "min_detectable_effect must be in (0, 1), got 0")]
    fn panics_on_confidence_first_min_detectable_effect_zero() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 0.95,
                min_detectable_effect: 0.0,
                power: 0.80,
            })
            .run();
    }

    #[test]
    #[should_panic(expected = "min_detectable_effect must be in (0, 1), got 1")]
    fn panics_on_confidence_first_min_detectable_effect_one() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 0.95,
                min_detectable_effect: 1.0,
                power: 0.80,
            })
            .run();
    }

    // --- Validation: coherence via builder API ---

    #[test]
    #[should_panic(expected = "REQUIRES_BASELINE")]
    fn panics_sample_size_first_without_baseline() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::SampleSizeFirst {
                samples: 100,
                confidence: 0.95,
            })
            // No spec_resolver or baseline_spec
            .run();
    }

    #[test]
    #[should_panic(expected = "REQUIRES_BASELINE_RATE")]
    fn panics_confidence_first_without_baseline() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ConfidenceFirst {
                confidence: 0.95,
                min_detectable_effect: 0.05,
                power: 0.80,
            })
            // No spec_resolver or baseline_spec
            .run();
    }

    // --- Feasibility via builder API ---

    #[test]
    #[should_panic(expected = "Infeasible")]
    fn panics_infeasible_verification() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 5,
                min_pass_rate: 0.95,
            })
            .intent(TestIntent::Verification)
            .run();
    }

    #[test]
    fn warns_infeasible_smoke() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 5,
                min_pass_rate: 0.95,
            })
            .intent(TestIntent::Smoke)
            .run();

        let warnings = result.verdict_record().warnings();
        assert!(
            warnings.iter().any(|w| w.code() == "UNDERSIZED"),
            "expected UNDERSIZED warning, got: {warnings:?}"
        );
    }

    #[test]
    fn feasible_verification_runs() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::new("test", &inputs, always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.90,
            })
            .intent(TestIntent::Verification)
            .run();
        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
    }

    // --- on_budget_exhausted setter precedence ---

    fn slow_success(_input: &str) -> TrialOutcome {
        std::thread::sleep(Duration::from_millis(5));
        TrialOutcome::success(Duration::from_millis(5))
    }

    #[test]
    fn explicit_execution_config_overrides_on_budget_exhausted_setter() {
        // An explicit ExecutionConfig is final: its exhaustion setting
        // wins over the builder's convenience setter. Documents the
        // precedence rule.
        let inputs = vec!["input".to_string()];
        let config = ExecutionConfig::new(100)
            .with_time_budget(Duration::from_millis(20))
            .with_on_budget_exhausted(BudgetExhaustedBehavior::Fail);

        let result = ProbabilisticTestBuilder::new("precedence", &inputs, slow_success)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.10,
            })
            .execution_config(config)
            // Setter asks for EvaluatePartial; the explicit config above
            // wins, so the test must still force-Fail rather than pass
            // on stats.
            .on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial)
            .run();

        assert_eq!(result.verdict_record().verdict(), Verdict::Fail);
        assert!(
            result
                .verdict_record()
                .warnings()
                .iter()
                .any(|w| w.code() == "BUDGET_EXHAUSTED"),
            "expected Fail-policy BUDGET_EXHAUSTED warning"
        );
    }
}
