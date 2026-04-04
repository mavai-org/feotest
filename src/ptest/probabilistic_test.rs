//! Streamlined builder for probabilistic tests.
//!
//! `ProbabilisticTest` detects the operational approach from the parameter
//! combination rather than requiring the developer to declare it upfront.
//! It auto-resolves baselines by use case ID and panics on fail, making the
//! common case — "does my service meet its threshold?" — a one-liner.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::controls::{ExecutionConfig, PacingConfig};
use crate::model::{TestIntent, ThresholdOrigin, TrialOutcome};
use crate::ptest::builder::ThresholdApproach;
use crate::ptest::runner;
use crate::spec::SpecResolver;
use crate::verdict::{Verdict, VerdictRecord};

/// A probabilistic test with approach detection from parameter combination.
///
/// The developer sets exactly two of the three statistical parameters
/// (samples, threshold, confidence) — or confidence + MDE + power — and
/// the framework derives the rest. Baselines are resolved automatically
/// from the use case ID.
///
/// # Examples
///
/// Threshold-first (simplest):
///
/// ```
/// use feotest::ptest::ProbabilisticTest;
/// use feotest::model::TrialOutcome;
/// use std::time::Duration;
///
/// let inputs = vec!["request".to_string()];
/// let record = ProbabilisticTest::new("my-service", &inputs, |_input| {
///     TrialOutcome::success(Duration::from_millis(1))
/// })
/// .samples(50)
/// .threshold(0.80)
/// .run();
/// ```
///
/// Sample-size-first (threshold derived from baseline):
///
/// ```ignore
/// let record = ProbabilisticTest::new("my-service", &inputs, |input| {
///     use_case.call(input)
/// })
/// .samples(200)
/// .confidence(0.95)
/// .run();
/// ```
pub struct ProbabilisticTest<'a, F> {
    use_case_id: String,
    inputs: &'a [String],
    trial: F,

    // Parameter triangle
    samples: Option<u32>,
    threshold: Option<f64>,
    confidence: Option<f64>,
    min_detectable_effect: Option<f64>,
    power: Option<f64>,

    // Baseline resolution
    baseline_path: Option<PathBuf>,
    baseline_dir: Option<PathBuf>,

    // Optional configuration
    intent: TestIntent,
    threshold_origin: ThresholdOrigin,
    contract_ref: Option<String>,
    transparent_stats: bool,
    time_budget: Option<Duration>,
    token_budget: Option<u64>,
    pacing: Option<PacingConfig>,
}

impl<'a, F> ProbabilisticTest<'a, F>
where
    F: FnMut(&str) -> TrialOutcome,
{
    /// Creates a new probabilistic test.
    ///
    /// # Arguments
    ///
    /// * `use_case_id` — identifies the use case; also used for baseline
    ///   resolution (`{use_case_id}.yaml`).
    /// * `inputs` — the input strings to cycle through during trials.
    /// * `trial` — closure that executes one trial and returns a
    ///   `TrialOutcome`.
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
            samples: None,
            threshold: None,
            confidence: None,
            min_detectable_effect: None,
            power: None,
            baseline_path: None,
            baseline_dir: None,
            intent: TestIntent::Verification,
            threshold_origin: ThresholdOrigin::Unspecified,
            contract_ref: None,
            transparent_stats: false,
            time_budget: None,
            token_budget: None,
            pacing: None,
        }
    }

    // --- Parameter triangle ---

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

    /// Sets the minimum detectable effect (used with confidence + power).
    #[must_use]
    pub const fn min_detectable_effect(mut self, mde: f64) -> Self {
        self.min_detectable_effect = Some(mde);
        self
    }

    /// Sets the statistical power (used with confidence + MDE).
    #[must_use]
    pub const fn power(mut self, p: f64) -> Self {
        self.power = Some(p);
        self
    }

    // --- Baseline resolution ---

    /// Sets an explicit baseline spec file path.
    ///
    /// When set, this overrides automatic resolution from the use case ID.
    #[must_use]
    pub fn baseline(mut self, path: impl Into<PathBuf>) -> Self {
        self.baseline_path = Some(path.into());
        self
    }

    /// Overrides the default baseline directory (`tests/baselines`).
    ///
    /// The framework will look for `{use_case_id}.yaml` in this directory.
    #[must_use]
    pub fn baseline_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.baseline_dir = Some(path.into());
        self
    }

    // --- Optional configuration ---

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

    /// Enables or disables transparent statistics in the verdict output.
    #[must_use]
    pub const fn transparent_stats(mut self, enabled: bool) -> Self {
        self.transparent_stats = enabled;
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
    pub const fn pacing(mut self, config: PacingConfig) -> Self {
        self.pacing = Some(config);
        self
    }

    // --- Execution ---

    /// Runs the probabilistic test.
    ///
    /// Detects the approach from the parameter combination, validates the
    /// parameter triangle, resolves the baseline if needed, executes the
    /// trials, and asserts the verdict.
    ///
    /// # Returns
    ///
    /// The `VerdictRecord` on pass.
    ///
    /// # Panics
    ///
    /// - If the parameter triangle is over-specified, under-specified,
    ///   or incomplete.
    /// - If a baseline is required but cannot be resolved.
    /// - If the verdict is `Fail`.
    pub fn run(self) -> VerdictRecord {
        let approach = self.detect_approach();
        let spec_resolver = self.build_spec_resolver();

        let config_overrides = self.build_execution_config(&approach);

        let result = runner::execute(
            &self.use_case_id,
            self.inputs,
            self.trial,
            &approach,
            self.intent,
            self.threshold_origin,
            self.contract_ref.as_deref(),
            spec_resolver.as_ref(),
            None, // baseline spec resolved via the resolver
            config_overrides.as_ref(),
        );

        let record = result.verdict_record();
        assert!(
            record.verdict() == Verdict::Pass,
            "\n\nProbabilistic test '{}' FAILED.\n\n\
             Pass rate: {:.4}\n\
             Verdict: {:?}\n\n\
             Use `ProbabilisticTestBuilder` directly if you need to inspect \
             a failing verdict without panicking.\n",
            self.use_case_id,
            record.functional().pass_rate(),
            record.verdict(),
        );

        result.verdict_record().clone()
    }

    /// Detects the threshold approach from the parameter combination.
    ///
    /// # Panics
    ///
    /// Panics with a clear message on over-specification, under-specification,
    /// or incomplete confidence-first parameters.
    fn detect_approach(&self) -> ThresholdApproach {
        let has_samples = self.samples.is_some();
        let has_threshold = self.threshold.is_some();
        let has_confidence = self.confidence.is_some();
        let has_mde = self.min_detectable_effect.is_some();
        let has_power = self.power.is_some();

        // Over-specification: samples + threshold + confidence
        assert!(
            !(has_samples && has_threshold && has_confidence),
            "\n\nOVER-SPECIFIED in ProbabilisticTest '{}':\n\n\
             samples, threshold, and confidence are all set.\n\
             Sample size, confidence, and threshold are mathematically linked.\n\
             You choose two; the framework derives the third.\n\n\
             Pick one approach:\n  \
             - Threshold-first:   .samples(n).threshold(rate)\n  \
             - Sample-size-first: .samples(n).confidence(level)\n  \
             - Confidence-first:  .confidence(level).min_detectable_effect(mde).power(p)\n",
            self.use_case_id
        );

        // Threshold-first: samples + threshold
        if has_samples && has_threshold && !has_confidence && !has_mde && !has_power {
            return ThresholdApproach::ThresholdFirst {
                samples: self.samples.unwrap(),
                min_pass_rate: self.threshold.unwrap(),
            };
        }

        // Sample-size-first: samples + confidence (no threshold)
        if has_samples && has_confidence && !has_threshold && !has_mde && !has_power {
            return ThresholdApproach::SampleSizeFirst {
                samples: self.samples.unwrap(),
                confidence: self.confidence.unwrap(),
            };
        }

        // Confidence-first: confidence + MDE + power
        if has_confidence && has_mde && has_power && !has_threshold {
            return ThresholdApproach::ConfidenceFirst {
                confidence: self.confidence.unwrap(),
                min_detectable_effect: self.min_detectable_effect.unwrap(),
                power: self.power.unwrap(),
            };
        }

        // Incomplete confidence-first: some but not all of confidence/MDE/power
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
                 Missing: {}\n\n\
                 How to fix:\n  \
                 - Add the missing parameter(s), OR\n  \
                 - Switch approach: .samples(n).threshold(rate) or .samples(n).confidence(level)\n",
                self.use_case_id,
                present.join(", "),
                missing.join(", "),
            );
        }

        // Under-specified: not enough parameters
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
             The parameter triangle requires at least two of: samples, threshold, confidence.\n\
             Or use confidence + min_detectable_effect + power for the confidence-first approach.\n\n\
             Valid combinations:\n  \
             - .samples(n).threshold(rate)\n  \
             - .samples(n).confidence(level)  (requires baseline)\n  \
             - .confidence(level).min_detectable_effect(mde).power(p)  (requires baseline)\n",
            self.use_case_id,
            if params_set.is_empty() {
                "(none)".to_string()
            } else {
                params_set.join(", ")
            },
        );
    }

    /// Builds the spec resolver for baseline resolution.
    ///
    /// If an explicit baseline path is set, returns a resolver that will
    /// find it. If a baseline directory is set, uses that. Otherwise,
    /// uses the default `tests/baselines` directory resolved from
    /// `CARGO_MANIFEST_DIR`.
    fn build_spec_resolver(&self) -> Option<SpecResolver> {
        // Only needed when there is no explicit threshold
        let needs_baseline = self.threshold.is_none();
        if !needs_baseline && self.baseline_path.is_none() {
            return None;
        }

        if let Some(ref path) = self.baseline_path {
            // Explicit baseline path: use its parent directory as the spec dir
            // and ensure the file name matches what the resolver expects.
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            return Some(SpecResolver::with_dir(parent));
        }

        if let Some(ref dir) = self.baseline_dir {
            return Some(SpecResolver::with_dir(dir));
        }

        // Default: tests/baselines resolved from CARGO_MANIFEST_DIR
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        let default_dir = PathBuf::from(manifest_dir).join("tests").join("baselines");
        Some(SpecResolver::with_dir(default_dir))
    }

    /// Builds `ExecutionConfig` from the approach and optional overrides.
    fn build_execution_config(&self, approach: &ThresholdApproach) -> Option<ExecutionConfig> {
        let has_overrides =
            self.time_budget.is_some() || self.token_budget.is_some() || self.pacing.is_some();

        if !has_overrides {
            return None;
        }

        let samples = match approach {
            ThresholdApproach::ThresholdFirst { samples, .. }
            | ThresholdApproach::SampleSizeFirst { samples, .. } => *samples,
            // For confidence-first, the runner will compute the sample size.
            // We cannot know it here, so we skip config overrides for that case
            // and let the runner build its own config.
            ThresholdApproach::ConfidenceFirst { .. } => return None,
        };

        let mut config = ExecutionConfig::new(samples);
        if let Some(budget) = self.time_budget {
            config = config.with_time_budget(budget);
        }
        if let Some(budget) = self.token_budget {
            config = config.with_token_budget(budget);
        }
        if let Some(ref pacing) = self.pacing {
            config = config.with_pacing(pacing.clone());
        }
        Some(config)
    }
}
