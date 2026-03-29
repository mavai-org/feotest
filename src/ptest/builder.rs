//! Builder for configuring and launching a probabilistic test.

use crate::controls::ExecutionConfig;
use crate::model::{TestIntent, ThresholdOrigin, TrialOutcome};
use crate::ptest::runner::{self, ProbabilisticTestResult};
use crate::spec::{BaselineSpec, SpecResolver};

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

    /// Enables transparent statistics in the verdict output.
    #[must_use]
    pub const fn transparent_stats(mut self, enabled: bool) -> Self {
        self.transparent_stats = enabled;
        self
    }

    /// Runs the probabilistic test and returns the result.
    ///
    /// # Panics
    ///
    /// Panics if no threshold approach has been set.
    pub fn run(self) -> ProbabilisticTestResult {
        let approach = self
            .approach
            .expect("threshold approach must be set before running");

        runner::execute(
            &self.use_case_id,
            self.inputs,
            self.trial,
            &approach,
            self.intent,
            self.threshold_origin,
            self.contract_ref.as_deref(),
            self.spec_resolver.as_ref(),
            self.baseline_spec,
            self.config_overrides.as_ref(),
        )
    }
}
