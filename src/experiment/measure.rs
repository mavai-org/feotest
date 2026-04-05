//! Measure experiment: establishing precise empirical baselines.

use crate::controls::{ExecutionConfig, PacingConfig, TokenRecorder};
use crate::experiment::engine::{ExecutionEngine, ExecutionResult};
use crate::model::TrialOutcome;
use crate::spec::SpecResolver;
use crate::spec::baseline::{BaselineSpec, RequirementsBlock, StatisticsBlock, SuccessRateBlock};
use crate::spec::namer::CovariateProfile;
use crate::usecase::UseCase;
use crate::spec::common::{
    build_cost_block, build_execution_block, build_failure_distribution, now_iso8601, round4,
    standard_error, wilson_interval, wilson_lower_bound,
};

/// A measure experiment that runs many samples to establish a precise baseline.
///
/// The resulting baseline spec contains the observed success rate, confidence
/// interval, and a derived minimum pass rate (Wilson lower bound at 95%
/// confidence).
///
/// # Examples
///
/// ```no_run
/// use feotest::experiment::MeasureExperiment;
/// use feotest::model::TrialOutcome;
/// use std::time::Duration;
///
/// let result = MeasureExperiment::new(
///     "my-service",
///     1000,
///     &["input-1".to_string(), "input-2".to_string()],
///     |input| TrialOutcome::success(Duration::from_millis(10)),
/// )
/// .run();
/// ```
pub struct MeasureExperiment<'a, F> {
    use_case_id: String,
    config: ExecutionConfig,
    inputs: &'a [String],
    trial: F,
    experiment_id: Option<String>,
    spec_resolver: Option<SpecResolver>,
    covariate_keys: Vec<String>,
    covariate_profile: CovariateProfile,
}

impl<'a, F> MeasureExperiment<'a, F>
where
    F: FnMut(&str) -> TrialOutcome,
{
    /// Creates a new measure experiment.
    pub fn new(
        use_case_id: impl Into<String>,
        samples: u32,
        inputs: &'a [String],
        trial: F,
    ) -> Self {
        Self {
            use_case_id: use_case_id.into(),
            config: ExecutionConfig::new(samples),
            inputs,
            trial,
            experiment_id: None,
            spec_resolver: None,
            covariate_keys: Vec::new(),
            covariate_profile: CovariateProfile::empty(),
        }
    }

    /// Creates a measure experiment from a use case, extracting identity
    /// and covariate information automatically.
    ///
    /// The use case provides:
    /// - The use case ID (for the spec filename and YAML body)
    /// - Covariate declarations (for the filename hash segments)
    /// - Resolved covariate values (for the YAML `covariates` block)
    ///
    /// The caller provides the trial function and inputs as usual.
    pub fn for_use_case(
        use_case: &dyn UseCase,
        samples: u32,
        inputs: &'a [String],
        trial: F,
    ) -> Self {
        let covariate_keys: Vec<String> = use_case
            .covariates()
            .iter()
            .map(|c| c.key().to_owned())
            .collect();
        let covariate_profile = use_case.resolve_covariates();

        Self {
            use_case_id: use_case.id().to_owned(),
            config: ExecutionConfig::new(samples),
            inputs,
            trial,
            experiment_id: None,
            spec_resolver: Some(SpecResolver::new("tests/baselines")),
            covariate_keys,
            covariate_profile,
        }
    }

    /// Sets the execution configuration (overrides sample count from constructor).
    #[must_use]
    pub const fn with_config(mut self, config: ExecutionConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the experiment identifier.
    #[must_use]
    pub fn with_experiment_id(mut self, id: impl Into<String>) -> Self {
        self.experiment_id = Some(id.into());
        self
    }

    /// Sets a spec resolver for writing the baseline spec to disk.
    #[must_use]
    pub fn with_spec_resolver(mut self, resolver: SpecResolver) -> Self {
        self.spec_resolver = Some(resolver);
        self
    }

    // --- Bare-name builder aliases (preferred API) ---

    /// Sets the experiment identifier.
    ///
    /// Bare-name alias for [`with_experiment_id`](Self::with_experiment_id).
    #[must_use]
    pub fn experiment_id(self, id: impl Into<String>) -> Self {
        self.with_experiment_id(id)
    }

    /// Sets the directory for writing the baseline spec.
    ///
    /// Convenience method that constructs a [`SpecResolver`] internally.
    #[must_use]
    pub fn spec_dir(self, path: impl Into<std::path::PathBuf>) -> Self {
        self.with_spec_resolver(SpecResolver::with_dir(path))
    }

    /// Sets the directory for writing the baseline spec.
    ///
    /// Preferred alias for [`spec_dir`](Self::spec_dir).
    #[must_use]
    pub fn baseline_dir(self, path: impl Into<std::path::PathBuf>) -> Self {
        self.spec_dir(path)
    }

    /// Sets the time budget for the experiment.
    ///
    /// The execution engine will stop once this wall-clock duration has elapsed.
    #[must_use]
    pub const fn time_budget(mut self, duration: std::time::Duration) -> Self {
        self.config = ExecutionConfig::set_time_budget(self.config, duration);
        self
    }

    /// Sets the token budget for the experiment.
    ///
    /// The execution engine will stop once this many tokens have been consumed.
    #[must_use]
    pub const fn token_budget(mut self, budget: u64) -> Self {
        self.config = ExecutionConfig::set_token_budget(self.config, budget);
        self
    }

    /// Sets pacing constraints for rate-limiting trial execution.
    #[must_use]
    pub const fn pacing(mut self, pacing_config: PacingConfig) -> Self {
        self.config = ExecutionConfig::set_pacing(self.config, pacing_config);
        self
    }

    /// Sets the covariate profile for the baseline filename.
    ///
    /// The covariate keys (declaration) and resolved values are encoded
    /// into the baseline filename, enabling the spec resolver to select
    /// the most appropriate baseline for the current test context.
    #[must_use]
    pub fn covariates(mut self, keys: Vec<String>, profile: CovariateProfile) -> Self {
        self.covariate_keys = keys;
        self.covariate_profile = profile;
        self
    }

    /// Runs the measure experiment and returns the result.
    pub fn run(mut self) -> MeasureResult {
        let token_recorder = TokenRecorder::new();
        let result =
            ExecutionEngine::run(&self.config, self.inputs, &token_recorder, &mut self.trial);

        let spec = self.build_spec(&result);

        // Write spec to disk if resolver is configured
        let cov_keys: Vec<&str> = self.covariate_keys.iter().map(String::as_str).collect();
        let spec_path = self
            .spec_resolver
            .as_ref()
            .and_then(|resolver| {
                resolver
                    .write(&spec, &cov_keys, &self.covariate_profile)
                    .ok()
            });

        MeasureResult {
            execution: result,
            spec,
            spec_path,
        }
    }

    fn build_spec(&self, result: &ExecutionResult) -> BaselineSpec {
        let summary = result.summary();
        let successes = summary.successes();
        let total = summary.samples_executed();
        let failures = summary.failures();

        let observed_rate = summary.observed_pass_rate();
        let (ci_lower, ci_upper) = wilson_interval(successes, total);
        let lower_bound = wilson_lower_bound(successes, total);
        let se = standard_error(successes, total);

        let mut spec = BaselineSpec::new(
            &self.use_case_id,
            now_iso8601(),
            build_execution_block(summary, self.config.samples()),
            RequirementsBlock {
                min_pass_rate: round4(lower_bound),
            },
            StatisticsBlock {
                success_rate: SuccessRateBlock {
                    observed: round4(observed_rate),
                    standard_error: round4(se),
                    confidence_interval95: [round4(ci_lower), round4(ci_upper)],
                },
                successes,
                failures,
                failure_distribution: build_failure_distribution(result.aggregate()),
            },
        );

        spec.experiment_id.clone_from(&self.experiment_id);
        spec.cost = Some(build_cost_block(summary.cost()));

        spec
    }
}

/// Result of a measure experiment.
#[derive(Debug)]
pub struct MeasureResult {
    execution: ExecutionResult,
    spec: BaselineSpec,
    spec_path: Option<std::path::PathBuf>,
}

impl MeasureResult {
    /// The execution result.
    #[must_use]
    pub const fn execution(&self) -> &ExecutionResult {
        &self.execution
    }

    /// The generated baseline spec.
    #[must_use]
    pub const fn spec(&self) -> &BaselineSpec {
        &self.spec
    }

    /// Path where the spec was written, if a resolver was configured.
    #[must_use]
    pub fn spec_path(&self) -> Option<&std::path::Path> {
        self.spec_path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn succeeding_trial(_input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    #[test]
    fn produces_baseline_spec() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new("test-service", 100, &inputs, succeeding_trial)
            .with_experiment_id("baseline-v1")
            .run();

        let spec = result.spec();
        assert_eq!(spec.use_case_id, "test-service");
        assert_eq!(spec.experiment_id.as_deref(), Some("baseline-v1"));
        assert_eq!(spec.statistics.successes, 100);
        assert_eq!(spec.statistics.failures, 0);
        assert!(spec.requirements.min_pass_rate > 0.9);
        assert!(spec.cost.is_some());
    }

    #[test]
    fn writes_spec_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());
        let inputs = vec!["input".to_string()];

        let result = MeasureExperiment::new("disk-test", 50, &inputs, succeeding_trial)
            .with_spec_resolver(resolver)
            .run();

        assert!(result.spec_path().is_some());
        assert!(result.spec_path().unwrap().exists());
    }

    #[test]
    fn tracks_failure_distribution() {
        let inputs = vec!["input".to_string()];
        let mut call_count = 0u32;
        let result = MeasureExperiment::new("mixed-service", 10, &inputs, |_input| {
            call_count += 1;
            if call_count % 3 == 0 {
                TrialOutcome::failure(
                    crate::model::ContractViolation::new("parse", "bad json"),
                    Duration::from_millis(1),
                )
            } else {
                TrialOutcome::success(Duration::from_millis(1))
            }
        })
        .run();

        let spec = result.spec();
        assert!(spec.statistics.failures > 0);
        assert!(spec.statistics.failure_distribution.is_some());
    }

    #[test]
    fn round4_works() {
        assert!((crate::spec::common::round4(0.123_456_789) - 0.1235).abs() < 1e-10);
    }
}
