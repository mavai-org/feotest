//! Measure experiment: establishing precise empirical baselines.

use crate::controls::{ExecutionConfig, PacingConfig, TokenRecorder};
use crate::experiment::engine::{ExecutionEngine, ExecutionResult};
use crate::model::TrialOutcome;
use crate::spec::SpecResolver;
use crate::spec::baseline::{BaselineSpec, ExpirationBlock, RequirementsBlock, StatisticsBlock};
use crate::spec::common::{
    build_cost_block, build_execution_block, build_failure_distribution,
    build_latency_distribution, build_success_rate_block, iso8601_plus_days, now_iso8601, round4,
    wilson_lower_bound,
};
use crate::spec::namer::CovariateProfile;
use crate::usecase::UseCase;

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
/// use feotest::usecase::UseCase;
/// use feotest::spec::namer::CovariateProfile;
/// use std::time::Duration;
///
/// struct MyService;
/// impl UseCase for MyService {
///     fn id(&self) -> &str { "my-service" }
/// }
///
/// let svc = MyService;
/// let inputs = vec!["input-1".to_string()];
/// let result = MeasureExperiment::new(
///     &svc,
///     1000,
///     &inputs,
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
    expires_in_days: u32,
}

impl<'a, F> MeasureExperiment<'a, F>
where
    F: FnMut(&str) -> TrialOutcome,
{
    /// Creates a new measure experiment.
    ///
    /// The use case provides:
    /// - The use case ID (for the spec filename and YAML body)
    /// - Covariate declarations (for the filename hash segments)
    /// - Resolved covariate values (for the YAML `covariates` block)
    ///
    /// The baseline spec is written to `tests/baselines/` by default.
    /// Override with [`.baseline_dir()`](Self::baseline_dir).
    pub fn new(use_case: &dyn UseCase, samples: u32, inputs: &'a [String], trial: F) -> Self {
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
            expires_in_days: 0,
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
    ///
    /// # Panics
    ///
    /// Panics if `duration` is zero.
    #[must_use]
    pub fn time_budget(mut self, duration: std::time::Duration) -> Self {
        self.config = ExecutionConfig::set_time_budget(self.config, duration);
        self
    }

    /// Sets the token budget for the experiment.
    ///
    /// The execution engine will stop once this many tokens have been consumed.
    ///
    /// # Panics
    ///
    /// Panics if `budget` is zero.
    #[must_use]
    pub fn token_budget(mut self, budget: u64) -> Self {
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

    /// Sets the baseline validity window in days.
    ///
    /// When non-zero, the baseline spec YAML carries an `expiration` block
    /// recording when the measurement ran and when it becomes stale.
    /// Probabilistic tests loading the spec consult this block via the
    /// [`crate::spec::expiration`] evaluator; expired baselines render a
    /// warning by default and can be escalated to a test failure via
    /// [`crate::ptest::ProbabilisticTestBuilder::fail_on_expired_baseline`].
    ///
    /// A value of `0` (the default) disables expiration entirely: no block
    /// is written, no checks are performed.
    #[must_use]
    pub const fn expires_in_days(mut self, days: u32) -> Self {
        self.expires_in_days = days;
        self
    }

    /// Runs the measure experiment and returns the result.
    pub fn run(mut self) -> MeasureResult {
        let token_recorder = TokenRecorder::new();
        let result = ExecutionEngine::run(
            &self.config,
            self.inputs,
            &token_recorder,
            None,
            &mut self.trial,
        );

        let spec = self.build_spec(&result);

        // Write spec to disk if resolver is configured
        let cov_keys: Vec<&str> = self.covariate_keys.iter().map(String::as_str).collect();
        let spec_path = self.spec_resolver.as_ref().and_then(|resolver| {
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
        let lower_bound = wilson_lower_bound(successes, total);

        let mut spec = BaselineSpec::new(
            &self.use_case_id,
            now_iso8601(),
            build_execution_block(summary, self.config.samples()),
            RequirementsBlock {
                min_pass_rate: round4(lower_bound),
            },
            StatisticsBlock {
                success_rate: build_success_rate_block(successes, total),
                successes,
                failures: summary.failures(),
                failure_distribution: build_failure_distribution(result.aggregate()),
                latency_distribution: build_latency_distribution(
                    result.aggregate().successful_latencies(),
                ),
            },
        );

        spec.experiment_id.clone_from(&self.experiment_id);
        spec.cost = Some(build_cost_block(summary.cost()));

        if self.expires_in_days > 0 {
            let baseline_end_time = spec.generated_at.clone();
            let expiration_date = iso8601_plus_days(&baseline_end_time, self.expires_in_days)
                .unwrap_or_else(|| baseline_end_time.clone());
            spec.expiration = Some(ExpirationBlock {
                expires_in_days: self.expires_in_days,
                baseline_end_time,
                expiration_date,
            });
        }

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

    struct TestUseCase {
        id: &'static str,
    }

    impl TestUseCase {
        const fn new(id: &'static str) -> Self {
            Self { id }
        }
    }

    impl UseCase for TestUseCase {
        fn id(&self) -> &str {
            self.id
        }
    }

    fn succeeding_trial(_input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    #[test]
    fn produces_baseline_spec() {
        let uc = TestUseCase::new("test-service");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 100, &inputs, succeeding_trial)
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
        let uc = TestUseCase::new("disk-test");
        let inputs = vec!["input".to_string()];

        let result = MeasureExperiment::new(&uc, 50, &inputs, succeeding_trial)
            .with_spec_resolver(SpecResolver::with_dir(dir.path()))
            .run();

        assert!(result.spec_path().is_some());
        assert!(result.spec_path().unwrap().exists());
    }

    #[test]
    fn tracks_failure_distribution() {
        let uc = TestUseCase::new("mixed-service");
        let inputs = vec!["input".to_string()];
        let mut call_count = 0u32;
        let result = MeasureExperiment::new(&uc, 10, &inputs, |_input| {
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

    #[test]
    fn no_experiment_id_produces_none() {
        let uc = TestUseCase::new("no-id");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 10, &inputs, succeeding_trial).run();
        assert!(result.spec().experiment_id.is_none());
    }

    #[test]
    fn bare_name_experiment_id_alias() {
        let uc = TestUseCase::new("alias-eid");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 10, &inputs, succeeding_trial)
            .experiment_id("v1")
            .run();
        assert_eq!(result.spec().experiment_id.as_deref(), Some("v1"));
    }

    #[test]
    fn bare_name_baseline_dir_alias() {
        let dir = tempfile::tempdir().unwrap();
        let uc = TestUseCase::new("alias-dir");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 10, &inputs, succeeding_trial)
            .baseline_dir(dir.path())
            .run();
        assert!(result.spec_path().is_some());
        assert!(result.spec_path().unwrap().exists());
    }

    #[test]
    fn spec_dir_alias() {
        let dir = tempfile::tempdir().unwrap();
        let uc = TestUseCase::new("alias-spec-dir");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 10, &inputs, succeeding_trial)
            .spec_dir(dir.path())
            .run();
        assert!(result.spec_path().is_some());
    }

    #[test]
    fn all_successes_has_empty_failure_distribution() {
        let uc = TestUseCase::new("all-pass");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 20, &inputs, succeeding_trial).run();

        let spec = result.spec();
        assert_eq!(spec.statistics.failures, 0);
        // No failure distribution when all succeed
        assert!(
            spec.statistics.failure_distribution.is_none()
                || spec
                    .statistics
                    .failure_distribution
                    .as_ref()
                    .unwrap()
                    .is_empty()
        );
    }

    #[test]
    fn cost_block_is_present() {
        let uc = TestUseCase::new("cost-test");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 10, &inputs, succeeding_trial).run();
        let cost = result.spec().cost.as_ref().unwrap();
        assert!(cost.total_time_ms > 0 || cost.avg_time_per_sample_ms == 0);
    }

    #[test]
    fn latency_distribution_captured() {
        let uc = TestUseCase::new("latency-cap");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 20, &inputs, succeeding_trial).run();
        // All succeed with 1ms latency — latency block should be present
        let latency = result.spec().statistics.latency_distribution.as_ref();
        assert!(latency.is_some());
        assert!(!latency.unwrap().latencies_ms.is_empty());
    }

    #[test]
    fn execution_result_accessible() {
        let uc = TestUseCase::new("exec-access");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 10, &inputs, succeeding_trial).run();
        assert_eq!(result.execution().summary().successes(), 10);
    }

    #[test]
    fn zero_expires_in_days_omits_expiration_block() {
        let uc = TestUseCase::new("no-expiry");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 10, &inputs, succeeding_trial).run();
        assert!(result.spec().expiration.is_none());
    }

    #[test]
    fn expires_in_days_populates_expiration_block() {
        let uc = TestUseCase::new("with-expiry");
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new(&uc, 10, &inputs, succeeding_trial)
            .expires_in_days(30)
            .run();

        let exp = result
            .spec()
            .expiration
            .as_ref()
            .expect("expiration block must be present");
        assert_eq!(exp.expires_in_days, 30);
        assert_eq!(exp.baseline_end_time, result.spec().generated_at);
        assert_eq!(
            exp.expiration_date,
            crate::spec::common::iso8601_plus_days(&exp.baseline_end_time, 30).unwrap()
        );
    }
}
