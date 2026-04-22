//! Streamlined builder for probabilistic tests.
//!
//! `ProbabilisticTest` detects the operational approach from the parameter
//! combination rather than requiring the developer to declare it upfront.
//! It auto-resolves baselines by use case ID and panics on fail, making the
//! common case — "does my service meet its threshold?" — a one-liner.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::controls::{ExecutionConfig, PacingConfig};
use crate::model::{BudgetExhaustedBehavior, TestIntent, ThresholdOrigin, TrialOutcome};
use crate::ptest::builder::ThresholdApproach;
use crate::ptest::runner::{self, AssessmentCriteria, BaselineContext};
use crate::spec::SpecResolver;
use crate::usecase::{CovariateContext, UseCase};
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
    covariate_context: Option<CovariateContext>,
    on_budget_exhausted: Option<BudgetExhaustedBehavior>,
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
            covariate_context: None,
            on_budget_exhausted: None,
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

    /// Sets the behaviour when a budget is exhausted.
    ///
    /// Defaults to [`BudgetExhaustedBehavior::Fail`]. Use
    /// [`BudgetExhaustedBehavior::EvaluatePartial`] for cost-constrained
    /// runs where a statistically valid verdict on the completed samples
    /// is preferable to failing outright.
    #[must_use]
    pub const fn on_budget_exhausted(mut self, behaviour: BudgetExhaustedBehavior) -> Self {
        self.on_budget_exhausted = Some(behaviour);
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
        crate::ptest::builder::validate_approach_bounds(&approach);

        // Resolver is built up-front so coherence validation can ask the
        // filesystem whether a baseline actually exists, rather than
        // optimistically assuming one will be there.
        let spec_resolver = self.build_spec_resolver();
        let has_baseline = self.check_baseline_available(spec_resolver.as_ref());

        let config = crate::ptest::builder::macro_config_from_approach(
            &self.use_case_id,
            &approach,
            self.threshold_origin,
            has_baseline,
        );
        crate::ptest::validation::validate(&config);

        let transparent_stats = self.transparent_stats;

        let config_overrides = self.build_execution_config(&approach);

        let criteria = AssessmentCriteria {
            approach,
            intent: self.intent,
            threshold_origin: self.threshold_origin,
            contract_ref: self.contract_ref,
            latency: crate::ptest::runner::LatencyConfig {
                thresholds: crate::latency::LatencyThresholds::new(),
                baseline_mode: None,
                baseline_confidence: crate::latency::DEFAULT_BASELINE_CONFIDENCE,
            },
            fail_on_expired_baseline: false,
            on_budget_exhausted: self.on_budget_exhausted,
        };
        let baseline = BaselineContext {
            spec_resolver,
            pre_resolved_spec: None,
            covariate_context: self.covariate_context,
        };

        let result = runner::execute(
            &self.use_case_id,
            self.inputs,
            self.trial,
            &criteria,
            baseline,
            config_overrides.as_ref(),
        );

        // Always print the brief verdict line
        let mut line = String::new();
        crate::reporting::transparent::render_verdict_line(result.verdict_record(), &mut line)
            .expect("formatting should not fail");
        eprintln!("{line}");

        // Write verdict XML to target/feotest/xml/ for the report pipeline
        write_verdict_xml(result.verdict_record());

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
        // A resolver is needed when:
        // - there is no explicit threshold (baseline required for derivation), OR
        // - covariates are declared (baseline must be loaded for integrity verification), OR
        // - the user explicitly asked for one by supplying a baseline path or directory.
        //
        // The last case lets coherence validation honestly detect a baseline
        // + explicit threshold conflict through the simplified API.
        let needs_baseline = self.threshold.is_none();
        let has_covariates = self.covariate_context.is_some();
        let user_specified_location = self.baseline_path.is_some() || self.baseline_dir.is_some();
        if !needs_baseline && !has_covariates && !user_specified_location {
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

    /// Reports whether a baseline spec is actually resolvable.
    ///
    /// Coherence validation relies on this being truthful: a threshold-less
    /// approach with no baseline on disk must fail early with the
    /// `REQUIRES_BASELINE[_RATE]` diagnostic, not cryptically inside the
    /// runner. Any resolution warnings surfaced here are discarded; the
    /// runner performs its own resolution and reports from there.
    fn check_baseline_available(&self, resolver: Option<&SpecResolver>) -> bool {
        let Some(resolver) = resolver else {
            return false;
        };
        if let Some(ref path) = self.baseline_path {
            return SpecResolver::resolve_file(path).is_ok();
        }
        let mut warnings = Vec::new();
        crate::ptest::baseline::resolve(
            resolver,
            &self.use_case_id,
            self.covariate_context.as_ref(),
            &mut warnings,
        )
        .is_some()
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
            config = config.pacing(pacing.clone());
        }
        if let Some(behaviour) = self.on_budget_exhausted {
            config = config.with_on_budget_exhausted(behaviour);
        }
        Some(config)
    }
}

/// Writes a verdict record to `target/feotest/xml/` as verdict XML.
///
/// Failures are silently ignored — verdict XML is a diagnostic side-effect,
/// not a test-critical path. A warning is printed to stderr if the write
/// fails.
fn write_verdict_xml(record: &crate::verdict::VerdictRecord) {
    use std::path::PathBuf;

    let use_case_id = record.identity().use_case_id();
    let test_name = record.identity().test_name().unwrap_or(use_case_id);

    let filename = format!("{use_case_id}.{test_name}.xml");
    let path = PathBuf::from("target/feotest/xml").join(filename);

    if let Err(e) = crate::reporting::VerdictXmlWriter::write_to_file(&path, record, None) {
        eprintln!(
            "feotest: warning: could not write verdict XML to {}: {e}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrialOutcome;
    use std::time::Duration;

    fn always_succeeds(_input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    // --- Valid approach detection ---

    #[test]
    fn detects_threshold_first() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(100)
            .threshold(0.90);
        let approach = pt.detect_approach();
        assert!(matches!(approach, ThresholdApproach::ThresholdFirst { .. }));
    }

    #[test]
    fn detects_sample_size_first() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(100)
            .confidence(0.95);
        let approach = pt.detect_approach();
        assert!(matches!(
            approach,
            ThresholdApproach::SampleSizeFirst { .. }
        ));
    }

    #[test]
    fn detects_confidence_first() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .confidence(0.95)
            .min_detectable_effect(0.05)
            .power(0.80);
        let approach = pt.detect_approach();
        assert!(matches!(
            approach,
            ThresholdApproach::ConfidenceFirst { .. }
        ));
    }

    // --- Invalid approach detection ---

    #[test]
    #[should_panic(expected = "OVER-SPECIFIED")]
    fn panics_over_specified() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(10)
            .threshold(0.90)
            .confidence(0.99)
            .run();
    }

    #[test]
    #[should_panic(expected = "UNDER-SPECIFIED")]
    fn panics_under_specified_samples_only() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(100)
            .run();
    }

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn panics_incomplete_confidence_first_missing_mde() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .confidence(0.99)
            .power(0.80)
            .run();
    }

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn panics_incomplete_confidence_first_missing_power() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .confidence(0.99)
            .min_detectable_effect(0.05)
            .run();
    }

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn panics_incomplete_confidence_first_missing_confidence() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .min_detectable_effect(0.05)
            .power(0.80)
            .run();
    }

    #[test]
    #[should_panic(expected = "UNDER-SPECIFIED")]
    fn panics_no_parameters() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds).run();
    }

    // --- Run-time behaviour ---

    #[test]
    fn run_returns_verdict_on_pass() {
        let inputs = vec!["input".to_string()];
        let record = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .threshold(0.80)
            .run();
        assert_eq!(record.verdict(), Verdict::Pass);
    }

    #[test]
    #[should_panic(expected = "FAILED")]
    fn run_panics_on_fail_verdict() {
        let inputs: Vec<String> = (0..10)
            .map(|i| if i < 8 { "fail".into() } else { "ok".into() })
            .collect();
        ProbabilisticTest::new("test", &inputs, |input| {
            if input == "fail" {
                TrialOutcome::failure(
                    crate::model::ContractViolation::new("check", "forced"),
                    Duration::from_millis(1),
                )
            } else {
                TrialOutcome::success(Duration::from_millis(1))
            }
        })
        .samples(100)
        .threshold(0.90)
        .run();
    }

    // --- Under-specified edge cases ---

    #[test]
    #[should_panic(expected = "UNDER-SPECIFIED")]
    fn panics_under_specified_threshold_only() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .threshold(0.90)
            .run();
    }

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn panics_incomplete_confidence_only() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .confidence(0.95)
            .run();
    }

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn panics_incomplete_mde_only() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .min_detectable_effect(0.05)
            .run();
    }

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn panics_incomplete_power_only() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .power(0.80)
            .run();
    }

    #[test]
    #[should_panic(expected = "INCOMPLETE")]
    fn panics_incomplete_mde_and_power_no_confidence() {
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("test", &inputs, always_succeeds)
            .min_detectable_effect(0.05)
            .power(0.80)
            .run();
    }

    // --- build_spec_resolver paths ---

    #[test]
    fn threshold_first_without_baseline_returns_no_resolver() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .threshold(0.90);
        assert!(pt.build_spec_resolver().is_none());
    }

    #[test]
    fn explicit_baseline_dir_uses_that_dir() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .confidence(0.95)
            .baseline_dir(dir.path());
        let resolver = pt.build_spec_resolver();
        assert!(resolver.is_some());
    }

    #[test]
    fn explicit_baseline_path_uses_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-spec.yaml");
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .confidence(0.95)
            .baseline(path);
        let resolver = pt.build_spec_resolver();
        assert!(resolver.is_some());
    }

    #[test]
    fn default_resolver_uses_tests_baselines() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .confidence(0.95);
        // No baseline_path or baseline_dir — falls through to default
        let resolver = pt.build_spec_resolver();
        assert!(resolver.is_some());
    }

    // --- build_execution_config paths ---

    #[test]
    fn no_overrides_returns_none() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .threshold(0.90);
        let approach = pt.detect_approach();
        assert!(pt.build_execution_config(&approach).is_none());
    }

    #[test]
    fn time_budget_returns_some_config() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .threshold(0.90)
            .time_budget(Duration::from_secs(60));
        let approach = pt.detect_approach();
        assert!(pt.build_execution_config(&approach).is_some());
    }

    #[test]
    fn token_budget_returns_some_config() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .threshold(0.90)
            .token_budget(10_000);
        let approach = pt.detect_approach();
        assert!(pt.build_execution_config(&approach).is_some());
    }

    #[test]
    fn confidence_first_with_overrides_returns_none() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .confidence(0.95)
            .min_detectable_effect(0.05)
            .power(0.80)
            .time_budget(Duration::from_secs(60));
        let approach = pt.detect_approach();
        // ConfidenceFirst cannot know sample size yet — returns None
        assert!(pt.build_execution_config(&approach).is_none());
    }

    // --- Optional builder methods ---

    #[test]
    fn intent_and_origin_propagate() {
        let inputs = vec!["input".to_string()];
        let record = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .threshold(0.80)
            .intent(TestIntent::Smoke)
            .threshold_origin(ThresholdOrigin::Sla)
            .contract_ref("SLA v1")
            .run();

        assert_eq!(record.intent(), TestIntent::Smoke);
        let prov = record.spec_provenance().unwrap();
        assert_eq!(prov.threshold_origin(), ThresholdOrigin::Sla);
        assert_eq!(prov.contract_ref(), Some("SLA v1"));
    }

    #[test]
    fn pacing_config_accepted() {
        let inputs = vec!["input".to_string()];
        let pt = ProbabilisticTest::new("test", &inputs, always_succeeds)
            .samples(50)
            .threshold(0.90)
            .pacing(crate::controls::PacingConfig::new().min_ms_per_sample(10));
        let approach = pt.detect_approach();
        assert!(pt.build_execution_config(&approach).is_some());
    }

    // --- Coherence rules through the simplified API ---

    struct NamedUseCase(&'static str);
    impl crate::usecase::UseCase for NamedUseCase {
        fn id(&self) -> &str {
            self.0
        }
    }

    #[test]
    #[should_panic(expected = "REQUIRES_BASELINE")]
    fn panics_sample_size_first_without_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("coherence-ssf-missing", &inputs, always_succeeds)
            .samples(200)
            .confidence(0.95)
            .baseline_dir(dir.path())
            .run();
    }

    #[test]
    #[should_panic(expected = "REQUIRES_BASELINE_RATE")]
    fn panics_confidence_first_without_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = vec!["input".to_string()];
        ProbabilisticTest::new("coherence-cf-missing", &inputs, always_succeeds)
            .confidence(0.95)
            .min_detectable_effect(0.05)
            .power(0.80)
            .baseline_dir(dir.path())
            .run();
    }

    #[test]
    #[should_panic(expected = "CONFLICT")]
    fn panics_conflict_non_normative_origin() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = vec!["input".to_string()];
        crate::experiment::MeasureExperiment::builder()
            .use_case_id("coherence-conflict")
            .use_case(|| ())
            .samples(200)
            .inputs(&inputs)
            .trial(|(): &(), input| always_succeeds(input))
            .baseline_dir(dir.path())
            .build()
            .run();

        ProbabilisticTest::new("coherence-conflict", &inputs, always_succeeds)
            .samples(100)
            .threshold(0.90)
            .threshold_origin(ThresholdOrigin::Empirical)
            .baseline_dir(dir.path())
            .run();
    }

    #[test]
    fn accepts_normative_override_sla() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = vec!["input".to_string()];
        crate::experiment::MeasureExperiment::builder()
            .use_case_id("coherence-normative-sla")
            .use_case(|| ())
            .samples(200)
            .inputs(&inputs)
            .trial(|(): &(), input| always_succeeds(input))
            .baseline_dir(dir.path())
            .build()
            .run();

        let record = ProbabilisticTest::new("coherence-normative-sla", &inputs, always_succeeds)
            .samples(100)
            .threshold(0.95)
            .threshold_origin(ThresholdOrigin::Sla)
            .baseline_dir(dir.path())
            .run();
        assert_eq!(record.verdict(), Verdict::Pass);
    }

    // --- on_budget_exhausted setter ---

    fn slow_success(_input: &str) -> TrialOutcome {
        std::thread::sleep(Duration::from_millis(5));
        TrialOutcome::success(Duration::from_millis(5))
    }

    #[test]
    fn on_budget_exhausted_setter_propagates() {
        // Default policy is Fail. Overriding to EvaluatePartial via the
        // new setter must flip the verdict from forced-Fail to the
        // stats-derived outcome (Pass at 100% pass rate vs 0.10 threshold).
        let inputs = vec!["input".to_string()];
        let record = ProbabilisticTest::new("budget-setter", &inputs, slow_success)
            .samples(100)
            .threshold(0.10)
            .time_budget(Duration::from_millis(20))
            .on_budget_exhausted(BudgetExhaustedBehavior::EvaluatePartial)
            .run();

        assert_eq!(record.verdict(), Verdict::Pass);
        let partial = record
            .warnings()
            .iter()
            .find(|w| w.code() == "BUDGET_EXHAUSTED_PARTIAL");
        assert!(
            partial.is_some(),
            "expected BUDGET_EXHAUSTED_PARTIAL warning, got {:?}",
            record.warnings()
        );
    }
}
