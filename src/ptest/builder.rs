//! Builder for configuring and launching a probabilistic test.
//!
//! The API shape matches [`crate::experiment::MeasureExperiment`]: the
//! use case id is explicit via [`use_case_id`](ProbabilisticTestBuilder::use_case_id),
//! the instance is produced by a factory via
//! [`use_case`](ProbabilisticTestBuilder::use_case), and the trial
//! closure receives `(&T, &str) -> TrialOutcome`. A probabilistic test
//! does not vary anything across samples, so the factory takes no
//! arguments — identical to measure's single-condition shape.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::controls::{ExecutionConfig, PacingConfig};
use crate::latency::{LatencyEnforcementMode, LatencyThresholds, Percentile};
use crate::model::{BudgetExhaustedBehavior, TestIntent, ThresholdOrigin, TrialOutcome};
use crate::ptest::probabilistic_test::ProbabilisticTest;
use crate::ptest::validation::MacroConfig;
use crate::spec::{BaselineSpec, SpecResolver};
use crate::usecase::{CovariateContext, UseCase};

type UseCaseFactory<'a, T> = Box<dyn Fn() -> T + 'a>;
type TrialClosure<'a, T> = Box<dyn FnMut(&T, &str) -> TrialOutcome + 'a>;

/// Configures the threshold derivation approach.
///
/// Exactly one approach must apply. When the user sets it explicitly
/// via [`ProbabilisticTestBuilder::approach`] the builder uses it as-is;
/// otherwise it is inferred from the parameter triangle
/// (`samples`/`threshold`/`confidence`/`min_detectable_effect`/`power`).
#[derive(Debug, Clone)]
pub enum ThresholdApproach {
    /// Fix samples and confidence; derive threshold from baseline spec.
    ///
    /// The threshold is the Wilson lower bound at the given confidence
    /// level.
    SampleSizeFirst {
        /// Number of test samples.
        samples: u32,
        /// Confidence level for threshold derivation.
        confidence: f64,
    },

    /// Fix confidence, effect size, and power; derive required sample
    /// count.
    ///
    /// The framework computes the minimum sample size needed to detect
    /// a degradation of `min_detectable_effect` with the given power.
    ConfidenceFirst {
        /// Required confidence level.
        confidence: f64,
        /// Smallest degradation worth detecting (absolute drop in pass
        /// rate).
        min_detectable_effect: f64,
        /// Probability of detecting a real degradation.
        power: f64,
    },

    /// Fix samples and an explicit threshold; framework derives implied
    /// confidence.
    ThresholdFirst {
        /// Number of test samples.
        samples: u32,
        /// Explicit minimum pass rate.
        min_pass_rate: f64,
    },
}

/// Fluent builder for a probabilistic test.
///
/// Required fields — `use_case_id`, `use_case` (factory), `inputs`,
/// `trial`, and an approach (either explicit via
/// [`approach`](Self::approach) or inferable from the parameter
/// triangle) — must be supplied before [`build`](Self::build) is
/// called. Missing any of them produces a panic naming the field and
/// the setter to call.
///
/// # Examples
///
/// Threshold-first, explicit approach:
///
/// ```
/// use feotest::ptest::ProbabilisticTestBuilder;
/// use feotest::ptest::builder::ThresholdApproach;
/// use feotest::model::TrialOutcome;
/// use feotest::verdict::Verdict;
/// use std::time::Duration;
///
/// let inputs = vec!["request".to_string()];
/// let result = ProbabilisticTestBuilder::builder()
///     .use_case_id("my-service")
///     .use_case(|| ())
///     .inputs(&inputs)
///     .trial(|(): &(), _input| TrialOutcome::success(Duration::from_millis(1)))
///     .approach(ThresholdApproach::ThresholdFirst {
///         samples: 50,
///         min_pass_rate: 0.90,
///     })
///     .build()
///     .run();
///
/// assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
/// ```
///
/// Threshold-first, inferred from the parameter triangle:
///
/// ```
/// use feotest::ptest::ProbabilisticTestBuilder;
/// use feotest::model::TrialOutcome;
/// use feotest::verdict::Verdict;
/// use std::time::Duration;
///
/// let inputs = vec!["request".to_string()];
/// let result = ProbabilisticTestBuilder::builder()
///     .use_case_id("my-service")
///     .use_case(|| ())
///     .inputs(&inputs)
///     .trial(|(): &(), _input| TrialOutcome::success(Duration::from_millis(1)))
///     .samples(50)
///     .threshold(0.90)
///     .build()
///     .run();
///
/// assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
/// ```
pub struct ProbabilisticTestBuilder<'a, T> {
    pub(crate) use_case_id: Option<String>,
    pub(crate) factory: Option<UseCaseFactory<'a, T>>,
    pub(crate) inputs: Option<&'a [String]>,
    pub(crate) trial: Option<TrialClosure<'a, T>>,

    pub(crate) approach: Option<ThresholdApproach>,
    pub(crate) samples: Option<u32>,
    pub(crate) threshold: Option<f64>,
    pub(crate) confidence: Option<f64>,
    pub(crate) min_detectable_effect: Option<f64>,
    pub(crate) power: Option<f64>,

    pub(crate) intent: TestIntent,
    pub(crate) threshold_origin: ThresholdOrigin,
    pub(crate) contract_ref: Option<String>,

    pub(crate) spec_resolver: Option<SpecResolver>,
    pub(crate) baseline_spec: Option<BaselineSpec>,
    pub(crate) baseline_path: Option<PathBuf>,
    pub(crate) baseline_dir: Option<PathBuf>,

    pub(crate) config_overrides: Option<ExecutionConfig>,
    pub(crate) time_budget: Option<Duration>,
    pub(crate) token_budget: Option<u64>,
    pub(crate) pacing: Option<PacingConfig>,
    pub(crate) on_budget_exhausted: Option<BudgetExhaustedBehavior>,

    pub(crate) transparent_stats: bool,
    pub(crate) covariate_context: Option<CovariateContext>,

    pub(crate) latency_thresholds: LatencyThresholds,
    pub(crate) baseline_latency_mode: Option<LatencyEnforcementMode>,
    pub(crate) baseline_latency_confidence: Option<f64>,
    pub(crate) fail_on_expired_baseline: bool,
}

impl<T> Default for ProbabilisticTestBuilder<'_, T> {
    fn default() -> Self {
        Self {
            use_case_id: None,
            factory: None,
            inputs: None,
            trial: None,
            approach: None,
            samples: None,
            threshold: None,
            confidence: None,
            min_detectable_effect: None,
            power: None,
            intent: TestIntent::Verification,
            threshold_origin: ThresholdOrigin::Unspecified,
            contract_ref: None,
            spec_resolver: None,
            baseline_spec: None,
            baseline_path: None,
            baseline_dir: None,
            config_overrides: None,
            time_budget: None,
            token_budget: None,
            pacing: None,
            on_budget_exhausted: None,
            transparent_stats: false,
            covariate_context: None,
            latency_thresholds: LatencyThresholds::new(),
            baseline_latency_mode: None,
            baseline_latency_confidence: None,
            fail_on_expired_baseline: false,
        }
    }
}

impl<'a> ProbabilisticTestBuilder<'a, ()> {
    /// Convenience constructor matching the pre-refactor positional
    /// signature. Internally wires the arguments into the measure-aligned
    /// builder shape: `T` is fixed to the unit type and the user's
    /// single-argument trial is adapted to `Fn(&(), &str)`.
    ///
    /// New code should prefer the explicit
    /// [`builder`](Self::builder) → setters pattern used by
    /// `MeasureExperiment`, `ExploreExperiment`, and
    /// `OptimizeExperiment`.
    ///
    /// # Panics
    ///
    /// Panics if `inputs` is empty (mirrors the old behaviour).
    pub fn new(
        use_case_id: impl Into<String>,
        inputs: &'a [String],
        mut trial: impl FnMut(&str) -> TrialOutcome + 'a,
    ) -> Self {
        Self::builder()
            .use_case_id(use_case_id)
            .use_case(|| ())
            .inputs(inputs)
            .trial(move |(): &(), input| trial(input))
    }
}

impl<'a, T> ProbabilisticTestBuilder<'a, T> {
    /// Starts a new builder.
    #[must_use]
    pub fn builder() -> Self {
        Self::default()
    }

    /// Builds and runs the test in one call.
    ///
    /// Equivalent to `self.build().run()`. Provided so code that
    /// pre-dates the `.build().run()` convention continues to read
    /// naturally.
    pub fn run(self) -> crate::ptest::ProbabilisticTestResult {
        self.build().run()
    }

    // --- required fields (measure-aligned) ---

    /// Sets the use case identifier.
    ///
    /// Appears in the verdict record and drives baseline resolution
    /// (`{use_case_id}.yaml`).
    #[must_use]
    pub fn use_case_id(mut self, id: impl Into<String>) -> Self {
        self.use_case_id = Some(id.into());
        self
    }

    /// Sets the use case factory.
    ///
    /// The factory is called once when [`run`](ProbabilisticTest::run)
    /// starts to produce the instance the trials are executed against.
    #[must_use]
    pub fn use_case(mut self, factory: impl Fn() -> T + 'a) -> Self {
        self.factory = Some(Box::new(factory));
        self
    }

    /// Sets the trial inputs.
    ///
    /// # Panics
    ///
    /// Panics if `inputs` is empty.
    #[must_use]
    pub fn inputs(mut self, inputs: &'a [String]) -> Self {
        assert!(!inputs.is_empty(), "inputs must not be empty");
        self.inputs = Some(inputs);
        self
    }

    /// Sets the trial closure.
    ///
    /// The closure receives a reference to the use case instance and
    /// an input string, and returns a [`TrialOutcome`]. It may borrow
    /// data that outlives the builder (the `'a` lifetime); it is not
    /// required to be `'static`.
    #[must_use]
    pub fn trial(mut self, trial: impl FnMut(&T, &str) -> TrialOutcome + 'a) -> Self {
        self.trial = Some(Box::new(trial));
        self
    }

    // --- parameter triangle (inferred approach) ---

    /// Fixes the sample count.
    #[must_use]
    pub const fn samples(mut self, n: u32) -> Self {
        self.samples = Some(n);
        self
    }

    /// Fixes the minimum pass rate threshold.
    #[must_use]
    pub const fn threshold(mut self, rate: f64) -> Self {
        self.threshold = Some(rate);
        self
    }

    /// Fixes the confidence level.
    #[must_use]
    pub const fn confidence(mut self, level: f64) -> Self {
        self.confidence = Some(level);
        self
    }

    /// Sets the minimum detectable effect (paired with confidence +
    /// power).
    #[must_use]
    pub const fn min_detectable_effect(mut self, mde: f64) -> Self {
        self.min_detectable_effect = Some(mde);
        self
    }

    /// Sets the statistical power (paired with confidence + MDE).
    #[must_use]
    pub const fn power(mut self, p: f64) -> Self {
        self.power = Some(p);
        self
    }

    /// Sets the threshold derivation approach explicitly.
    ///
    /// When set, overrides any inference from the parameter triangle.
    #[must_use]
    pub fn approach(mut self, approach: ThresholdApproach) -> Self {
        self.approach = Some(approach);
        self
    }

    // --- baseline resolution ---

    /// Sets an explicit baseline spec file path.
    #[must_use]
    pub fn baseline(mut self, path: impl Into<PathBuf>) -> Self {
        self.baseline_path = Some(path.into());
        self
    }

    /// Overrides the default baseline directory (`tests/baselines`).
    ///
    /// The framework looks for `{use_case_id}.yaml` in this directory.
    #[must_use]
    pub fn baseline_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.baseline_dir = Some(path.into());
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

    /// Sets the spec resolver for baseline-driven threshold derivation.
    #[must_use]
    pub fn spec_resolver(mut self, resolver: SpecResolver) -> Self {
        self.spec_resolver = Some(resolver);
        self
    }

    // --- optional configuration ---

    /// Sets the test intent.
    #[must_use]
    pub const fn intent(mut self, intent: TestIntent) -> Self {
        self.intent = intent;
        self
    }

    /// Sets the threshold origin (provenance).
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

    /// Enables transparent statistics in the verdict output.
    #[must_use]
    pub const fn transparent_stats(mut self, enabled: bool) -> Self {
        self.transparent_stats = enabled;
        self
    }

    /// Overrides execution configuration (warmup, budgets, pacing).
    #[must_use]
    pub fn execution_config(mut self, config: ExecutionConfig) -> Self {
        self.config_overrides = Some(config);
        self
    }

    /// Sets a wall-clock time budget for the test.
    #[must_use]
    pub const fn time_budget(mut self, budget: Duration) -> Self {
        self.time_budget = Some(budget);
        self
    }

    /// Sets a token budget for the test.
    #[must_use]
    pub const fn token_budget(mut self, budget: u64) -> Self {
        self.token_budget = Some(budget);
        self
    }

    /// Sets pacing constraints for rate-limiting trial execution.
    #[must_use]
    pub fn pacing(mut self, config: PacingConfig) -> Self {
        self.pacing = Some(config);
        self
    }

    /// Sets the behaviour when a budget is exhausted.
    ///
    /// If a full [`ExecutionConfig`] is also supplied via
    /// [`execution_config`](Self::execution_config), that config's own
    /// setting wins — this setter only has effect when the runner
    /// synthesises a default config.
    #[must_use]
    pub const fn on_budget_exhausted(mut self, behaviour: BudgetExhaustedBehavior) -> Self {
        self.on_budget_exhausted = Some(behaviour);
        self
    }

    /// Sets covariate context from a use case for baseline selection.
    ///
    /// When set, the resolver uses covariate-aware selection to find
    /// the best-matching baseline rather than returning the first
    /// match. If the use case declares no covariates, this is a no-op.
    #[must_use]
    pub fn covariate_source(mut self, use_case: &dyn UseCase) -> Self {
        self.covariate_context = CovariateContext::from_use_case(use_case);
        self
    }

    // --- latency configuration ---

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

    /// Controls whether baseline-derived latency thresholds fail the
    /// verdict on violation (`true` → `Strict`) or warn only
    /// (`false` → `Advisory`).
    #[must_use]
    pub const fn enforce_baseline_latency(mut self, strict: bool) -> Self {
        self.baseline_latency_mode = Some(if strict {
            LatencyEnforcementMode::Strict
        } else {
            LatencyEnforcementMode::Advisory
        });
        self
    }

    /// Overrides the confidence level used when deriving a latency
    /// threshold from the baseline (default `0.95`).
    #[must_use]
    pub const fn baseline_latency_confidence(mut self, confidence: f64) -> Self {
        self.baseline_latency_confidence = Some(confidence);
        self
    }

    /// Escalates expired baselines from warning to test failure.
    #[must_use]
    pub const fn fail_on_expired_baseline(mut self, fail: bool) -> Self {
        self.fail_on_expired_baseline = fail;
        self
    }

    // --- terminal ---

    /// Builds the probabilistic test.
    ///
    /// # Panics
    ///
    /// Panics if any required field (`use_case_id`, `use_case` factory,
    /// `inputs`, `trial`) is missing, or if the parameter triangle is
    /// over-specified / under-specified / incomplete for any
    /// [`ThresholdApproach`] and no explicit `.approach(...)` was set.
    #[must_use]
    pub fn build(mut self) -> ProbabilisticTest<'a, T> {
        // Required-field checks run first so callers get the clearest
        // possible panic message (naming the missing setter) before any
        // approach inference is attempted.
        let use_case_id = self
            .use_case_id
            .expect("use_case_id must be set via .use_case_id(...)");
        let factory = self
            .factory
            .expect("use_case factory must be set via .use_case(...)");
        let inputs = self.inputs.expect("inputs must be set via .inputs(...)");
        let trial = self.trial.expect("trial must be set via .trial(...)");

        let approach = self.approach.take().unwrap_or_else(|| {
            detect_approach_from_triangle(
                &use_case_id,
                self.samples,
                self.threshold,
                self.confidence,
                self.min_detectable_effect,
                self.power,
            )
        });
        validate_approach_bounds(&approach);

        ProbabilisticTest {
            use_case_id,
            factory,
            inputs,
            trial,
            approach,
            intent: self.intent,
            threshold_origin: self.threshold_origin,
            contract_ref: self.contract_ref,
            spec_resolver: self.spec_resolver,
            baseline_spec: self.baseline_spec,
            baseline_path: self.baseline_path,
            baseline_dir: self.baseline_dir,
            config_overrides: self.config_overrides,
            time_budget: self.time_budget,
            token_budget: self.token_budget,
            pacing: self.pacing,
            on_budget_exhausted: self.on_budget_exhausted,
            transparent_stats: self.transparent_stats,
            covariate_context: self.covariate_context,
            latency_thresholds: self.latency_thresholds,
            baseline_latency_mode: self.baseline_latency_mode,
            baseline_latency_confidence: self.baseline_latency_confidence,
            fail_on_expired_baseline: self.fail_on_expired_baseline,
        }
    }
}

/// Builds an approach from the parameter triangle.
///
/// # Panics
///
/// Panics on over-specification, under-specification, or incomplete
/// confidence-first parameters.
fn detect_approach_from_triangle(
    use_case_id: &str,
    samples: Option<u32>,
    threshold: Option<f64>,
    confidence: Option<f64>,
    mde: Option<f64>,
    power: Option<f64>,
) -> ThresholdApproach {
    let has_samples = samples.is_some();
    let has_threshold = threshold.is_some();
    let has_confidence = confidence.is_some();
    let has_mde = mde.is_some();
    let has_power = power.is_some();

    assert!(
        !(has_samples && has_threshold && has_confidence),
        "\n\nOVER-SPECIFIED in ProbabilisticTest '{use_case_id}':\n\n\
         samples, threshold, and confidence are all set.\n\
         Sample size, confidence, and threshold are mathematically linked.\n\
         You choose two; the framework derives the third.\n\n\
         Pick one approach:\n  \
         - Threshold-first:   .samples(n).threshold(rate)\n  \
         - Sample-size-first: .samples(n).confidence(level)\n  \
         - Confidence-first:  .confidence(level).min_detectable_effect(mde).power(p)\n"
    );

    if has_samples && has_threshold && !has_confidence && !has_mde && !has_power {
        return ThresholdApproach::ThresholdFirst {
            samples: samples.unwrap(),
            min_pass_rate: threshold.unwrap(),
        };
    }

    if has_samples && has_confidence && !has_threshold && !has_mde && !has_power {
        return ThresholdApproach::SampleSizeFirst {
            samples: samples.unwrap(),
            confidence: confidence.unwrap(),
        };
    }

    if has_confidence && has_mde && has_power && !has_threshold {
        return ThresholdApproach::ConfidenceFirst {
            confidence: confidence.unwrap(),
            min_detectable_effect: mde.unwrap(),
            power: power.unwrap(),
        };
    }

    let cf_count = [has_confidence, has_mde, has_power]
        .iter()
        .filter(|&&v| v)
        .count();
    if cf_count > 0 && cf_count < 3 && !has_samples && !has_threshold {
        let mut present = Vec::new();
        let mut missing = Vec::new();
        if has_confidence {
            present.push("confidence");
        } else {
            missing.push("confidence");
        }
        if has_mde {
            present.push("min_detectable_effect");
        } else {
            missing.push("min_detectable_effect");
        }
        if has_power {
            present.push("power");
        } else {
            missing.push("power");
        }
        panic!(
            "\n\nINCOMPLETE in ProbabilisticTest '{}':\n\n\
             The confidence-first approach requires all three parameters:\n  \
             confidence, min_detectable_effect, and power.\n\n\
             Present: {}\n\
             Missing: {}\n",
            use_case_id,
            present.join(", "),
            missing.join(", "),
        );
    }

    let mut params_set = Vec::new();
    if has_samples {
        params_set.push("samples");
    }
    if has_threshold {
        params_set.push("threshold");
    }
    if has_confidence {
        params_set.push("confidence");
    }
    if has_mde {
        params_set.push("min_detectable_effect");
    }
    if has_power {
        params_set.push("power");
    }

    panic!(
        "\n\nUNDER-SPECIFIED in ProbabilisticTest '{}':\n\n\
         Parameters set: {}\n\n\
         Set an explicit .approach(...) or supply at least two of:\n  \
         samples, threshold, confidence (or confidence + min_detectable_effect + power).\n",
        use_case_id,
        if params_set.is_empty() {
            "(none)".to_string()
        } else {
            params_set.join(", ")
        },
    );
}

/// Validates that the approach parameters fall within mathematical
/// constraints.
///
/// # Panics
///
/// Panics with a descriptive message if a rate or confidence is outside
/// `(0, 1)` or similar.
pub fn validate_approach_bounds(approach: &ThresholdApproach) {
    match approach {
        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        } => {
            assert!(*samples > 0, "samples must be positive, got 0");
            assert!(
                (0.0..=1.0).contains(min_pass_rate) && min_pass_rate.is_finite(),
                "min_pass_rate must be in [0, 1], got {min_pass_rate}"
            );
        }
        ThresholdApproach::SampleSizeFirst {
            samples,
            confidence,
        } => {
            assert!(*samples > 0, "samples must be positive, got 0");
            assert!(
                (0.0..1.0).contains(confidence) && confidence.is_finite(),
                "confidence must be in (0, 1), got {confidence}"
            );
        }
        ThresholdApproach::ConfidenceFirst {
            confidence,
            min_detectable_effect,
            power,
        } => {
            assert!(
                (0.0..1.0).contains(confidence) && confidence.is_finite(),
                "confidence must be in (0, 1), got {confidence}"
            );
            assert!(
                min_detectable_effect.is_finite() && *min_detectable_effect > 0.0,
                "min_detectable_effect must be positive, got {min_detectable_effect}"
            );
            assert!(
                (0.0..1.0).contains(power) && power.is_finite(),
                "power must be in (0, 1), got {power}"
            );
        }
    }
}

/// Constructs a `MacroConfig` from a `ThresholdApproach` for coherence
/// validation.
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

/// Resolves the default baseline directory path from `CARGO_MANIFEST_DIR`.
pub(crate) fn default_baseline_dir() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(manifest_dir).join("tests").join("baselines")
}

/// Builds a spec resolver from `baseline_path` / `baseline_dir` /
/// default.
pub(crate) fn build_default_spec_resolver(
    baseline_path: Option<&Path>,
    baseline_dir: Option<&Path>,
) -> SpecResolver {
    if let Some(path) = baseline_path {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        return SpecResolver::with_dir(parent);
    }
    if let Some(dir) = baseline_dir {
        return SpecResolver::with_dir(dir);
    }
    SpecResolver::with_dir(default_baseline_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrialOutcome;
    use crate::verdict::Verdict;
    use std::time::Duration;

    fn always_succeeds(_uc: &(), _input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    #[test]
    fn threshold_first_explicit_passes() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::builder()
            .use_case_id("ssf-explicit")
            .use_case(|| ())
            .inputs(&inputs)
            .trial(always_succeeds)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 30,
                min_pass_rate: 0.80,
            })
            .build()
            .run();

        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
    }

    #[test]
    fn threshold_first_inferred_from_triangle() {
        let inputs = vec!["input".to_string()];
        let result = ProbabilisticTestBuilder::builder()
            .use_case_id("tri-inferred")
            .use_case(|| ())
            .inputs(&inputs)
            .trial(always_succeeds)
            .samples(30)
            .threshold(0.80)
            .build()
            .run();

        assert_eq!(result.verdict_record().verdict(), Verdict::Pass);
    }

    #[test]
    #[should_panic(expected = "OVER-SPECIFIED")]
    fn over_specified_triangle_panics() {
        let inputs = vec!["input".to_string()];
        let _ = ProbabilisticTestBuilder::builder()
            .use_case_id("over")
            .use_case(|| ())
            .inputs(&inputs)
            .trial(always_succeeds)
            .samples(30)
            .threshold(0.80)
            .confidence(0.95)
            .build();
    }

    #[test]
    #[should_panic(expected = "UNDER-SPECIFIED")]
    fn under_specified_triangle_panics() {
        let inputs = vec!["input".to_string()];
        let _ = ProbabilisticTestBuilder::builder()
            .use_case_id("under")
            .use_case(|| ())
            .inputs(&inputs)
            .trial(always_succeeds)
            .build();
    }

    #[test]
    #[should_panic(expected = "use_case_id must be set")]
    fn build_without_any_required_fields_panics() {
        let _ = ProbabilisticTestBuilder::<()>::builder().build();
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn rejects_empty_inputs() {
        let empty: Vec<String> = vec![];
        let _ = ProbabilisticTestBuilder::<()>::builder().inputs(&empty);
    }
}
