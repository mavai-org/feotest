//! Measure experiment: establishing precise empirical baselines.

use std::path::PathBuf;
use std::time::Duration;

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

type ServiceContractFactory<'a, T> = Box<dyn Fn() -> T + 'a>;
type TrialClosure<'a, T> = Box<dyn Fn(&T, &str) -> TrialOutcome + 'a>;

/// Default directory for baseline spec output.
const DEFAULT_BASELINE_DIR: &str = "tests/baselines";

/// A measure experiment that runs many samples to establish a precise baseline.
///
/// The resulting baseline spec contains the observed success rate, confidence
/// interval, and a derived minimum pass rate (Wilson lower bound at 95%
/// confidence).
///
/// Construct via [`MeasureExperiment::builder`]; there is no public
/// constructor.
///
/// The API shape matches [`super::ExploreExperiment`] and
/// [`super::OptimizeExperiment`]: the service contract id is explicit via
/// [`service_contract_id`](MeasureExperimentBuilder::service_contract_id), the instance
/// is produced by a factory closure set via
/// [`service_contract`](MeasureExperimentBuilder::service_contract), and the trial
/// closure receives a reference to the produced instance. Measure's
/// factory takes no arguments because the experiment measures a single
/// condition; explore and optimize pass a factor into theirs.
///
/// # Examples
///
/// ```no_run
/// use feotest::experiment::MeasureExperiment;
/// use feotest::model::TrialOutcome;
/// use std::time::Duration;
///
/// struct MyService;
/// impl MyService {
///     fn call(&self, _instruction: &str) -> TrialOutcome {
///         TrialOutcome::success(Duration::from_millis(10))
///     }
/// }
///
/// let inputs = vec!["input-1".to_string()];
/// let result = MeasureExperiment::builder()
///     .service_contract_id("my-service")
///     .service_contract(|| MyService)
///     .samples(1000)
///     .inputs(&inputs)
///     .trial(|uc: &MyService, input| uc.call(input))
///     .build()
///     .run();
/// ```
pub struct MeasureExperiment<'a, T> {
    service_contract_id: String,
    factory: ServiceContractFactory<'a, T>,
    config: ExecutionConfig,
    inputs: &'a [String],
    trial: TrialClosure<'a, T>,
    experiment_id: Option<String>,
    spec_resolver: Option<SpecResolver>,
    covariate_keys: Vec<String>,
    covariate_profile: CovariateProfile,
    expires_in_days: u32,
}

impl<'a, T> MeasureExperiment<'a, T> {
    /// Starts a new builder for a measure experiment.
    ///
    /// Required fields (`service_contract_id`, `service_contract` factory, `samples`,
    /// `inputs`, `trial`) must be set via their corresponding setters
    /// before [`build`](MeasureExperimentBuilder::build) is called.
    /// Optional fields carry documented defaults.
    #[must_use]
    pub fn builder() -> MeasureExperimentBuilder<'a, T> {
        MeasureExperimentBuilder::default()
    }

    /// Runs the measure experiment and returns the result.
    ///
    /// The result is often discarded — callers commonly want only the
    /// side effect of writing the baseline spec to disk — so this
    /// method is deliberately **not** `#[must_use]`.
    pub fn run(self) -> MeasureResult {
        let service_contract = (self.factory)();

        let token_recorder = TokenRecorder::new();
        let trial = self.trial;
        let mut trial_fn = |input: &str| (trial)(&service_contract, input);

        let result = ExecutionEngine::run(
            &self.config,
            self.inputs,
            &token_recorder,
            crate::controls::run::current(),
            &mut trial_fn,
        );

        let spec = build_spec(
            &self.service_contract_id,
            &self.config,
            &result,
            self.experiment_id.as_deref(),
            self.expires_in_days,
        );

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
}

fn build_spec(
    service_contract_id: &str,
    config: &ExecutionConfig,
    result: &ExecutionResult,
    experiment_id: Option<&str>,
    expires_in_days: u32,
) -> BaselineSpec {
    let summary = result.summary();
    let successes = summary.successes();
    let total = summary.samples_executed();
    let lower_bound = wilson_lower_bound(successes, total);

    let mut spec = BaselineSpec::new(
        service_contract_id,
        now_iso8601(),
        build_execution_block(summary, config.samples()),
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

    spec.experiment_id = experiment_id.map(ToOwned::to_owned);
    spec.cost = Some(build_cost_block(summary.cost()));

    if expires_in_days > 0 {
        let baseline_end_time = spec.generated_at.clone();
        let expiration_date = iso8601_plus_days(&baseline_end_time, expires_in_days)
            .unwrap_or_else(|| baseline_end_time.clone());
        spec.expiration = Some(ExpirationBlock {
            expires_in_days,
            baseline_end_time,
            expiration_date,
        });
    }

    spec
}

/// Fluent builder for [`MeasureExperiment`].
///
/// Required fields — `service_contract_id`, `service_contract` (factory), `samples`,
/// `inputs`, and `trial` — must be set before [`build`](Self::build)
/// is called. Missing any of them produces a panic naming the field
/// and the setter to call.
///
/// Optional fields carry documented defaults. Setters that validate a
/// single value (e.g., positive sample count, non-empty inputs) panic
/// at the setter rather than deferring to `build`.
pub struct MeasureExperimentBuilder<'a, T> {
    service_contract_id: Option<String>,
    factory: Option<ServiceContractFactory<'a, T>>,
    samples: Option<u32>,
    inputs: Option<&'a [String]>,
    trial: Option<TrialClosure<'a, T>>,
    experiment_id: Option<String>,
    spec_resolver: SpecResolver,
    covariate_keys: Vec<String>,
    covariate_profile: CovariateProfile,
    expires_in_days: u32,
    time_budget: Option<Duration>,
    token_budget: Option<u64>,
    pacing: Option<PacingConfig>,
}

impl<T> Default for MeasureExperimentBuilder<'_, T> {
    fn default() -> Self {
        Self {
            service_contract_id: None,
            factory: None,
            samples: None,
            inputs: None,
            trial: None,
            experiment_id: None,
            spec_resolver: SpecResolver::new(DEFAULT_BASELINE_DIR),
            covariate_keys: Vec::new(),
            covariate_profile: CovariateProfile::empty(),
            expires_in_days: 0,
            time_budget: None,
            token_budget: None,
            pacing: None,
        }
    }
}

impl<'a, T> MeasureExperimentBuilder<'a, T> {
    // --- required fields ---

    /// Sets the service contract identifier.
    ///
    /// Appears in the baseline spec YAML and in the spec resolver's
    /// output path.
    #[must_use]
    pub fn service_contract_id(mut self, id: impl Into<String>) -> Self {
        self.service_contract_id = Some(id.into());
        self
    }

    /// Sets the service contract factory.
    ///
    /// The factory is called once at the start of
    /// [`run`](MeasureExperiment::run) to produce the service contract instance
    /// the experiment measures. The instance is owned by the
    /// experiment, referenced by the trial closure on every sample,
    /// and dropped when the run completes.
    #[must_use]
    pub fn service_contract(mut self, factory: impl Fn() -> T + 'a) -> Self {
        self.factory = Some(Box::new(factory));
        self
    }

    /// Sets the number of samples to run.
    ///
    /// # Panics
    ///
    /// Panics if `samples` is zero.
    #[must_use]
    pub fn samples(mut self, samples: u32) -> Self {
        assert!(samples > 0, "samples must be positive, got 0");
        self.samples = Some(samples);
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
    /// The closure receives a reference to the service contract instance
    /// produced by the factory and an input string, and returns a
    /// [`TrialOutcome`]. It may borrow data that outlives the builder
    /// (the `'a` lifetime); it is not required to be `'static`.
    #[must_use]
    pub fn trial(mut self, trial: impl Fn(&T, &str) -> TrialOutcome + 'a) -> Self {
        self.trial = Some(Box::new(trial));
        self
    }

    // --- optional fields ---

    /// Sets the experiment identifier. Default: none.
    #[must_use]
    pub fn experiment_id(mut self, id: impl Into<String>) -> Self {
        self.experiment_id = Some(id.into());
        self
    }

    /// Sets the directory for writing the baseline spec.
    ///
    /// Default: `tests/baselines`.
    #[must_use]
    pub fn baseline_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.spec_resolver = SpecResolver::with_dir(path);
        self
    }

    /// Sets a spec resolver for writing the baseline spec to disk.
    ///
    /// This is the power-user escape hatch; prefer
    /// [`baseline_dir`](Self::baseline_dir) for the common "just set a
    /// path" case.
    #[must_use]
    pub fn spec_resolver(mut self, resolver: SpecResolver) -> Self {
        self.spec_resolver = resolver;
        self
    }

    /// Sets the time budget for the experiment.
    ///
    /// The execution engine will stop once this wall-clock duration
    /// has elapsed.
    ///
    /// # Panics
    ///
    /// Panics if `duration` is zero.
    #[must_use]
    pub fn time_budget(mut self, duration: Duration) -> Self {
        assert!(!duration.is_zero(), "time_budget must be positive");
        self.time_budget = Some(duration);
        self
    }

    /// Sets the token budget for the experiment.
    ///
    /// The execution engine will stop once this many tokens have been
    /// consumed.
    ///
    /// # Panics
    ///
    /// Panics if `budget` is zero.
    #[must_use]
    pub fn token_budget(mut self, budget: u64) -> Self {
        assert!(budget > 0, "token_budget must be positive, got 0");
        self.token_budget = Some(budget);
        self
    }

    /// Sets pacing constraints for rate-limiting trial execution.
    #[must_use]
    pub const fn pacing(mut self, pacing_config: PacingConfig) -> Self {
        self.pacing = Some(pacing_config);
        self
    }

    /// Sets the declared covariate keys and resolved profile for this
    /// baseline.
    ///
    /// These determine the baseline filename and let
    /// [`crate::spec::SpecResolver`] select the appropriate baseline
    /// for a given test context. Default: empty — no covariate
    /// dimensions are recorded.
    #[must_use]
    pub fn covariates(mut self, keys: Vec<String>, profile: CovariateProfile) -> Self {
        self.covariate_keys = keys;
        self.covariate_profile = profile;
        self
    }

    /// Sets the baseline validity window in days.
    ///
    /// When non-zero, the baseline spec YAML carries an `expiration`
    /// block recording when the measurement ran and when it becomes
    /// stale. Probabilistic tests loading the spec consult this block
    /// via the [`crate::spec::expiration`] evaluator; expired
    /// baselines render a warning by default and can be escalated to a
    /// test failure via
    /// [`crate::ptest::ProbabilisticTestBuilder::fail_on_expired_baseline`].
    ///
    /// A value of `0` (the default) disables expiration entirely: no
    /// block is written, no checks are performed.
    #[must_use]
    pub const fn expires_in_days(mut self, days: u32) -> Self {
        self.expires_in_days = days;
        self
    }

    /// Builds the [`MeasureExperiment`].
    ///
    /// # Panics
    ///
    /// Panics if any required field (`service_contract_id`, `service_contract` factory,
    /// `samples`, `inputs`, `trial`) is missing.
    #[must_use]
    pub fn build(self) -> MeasureExperiment<'a, T> {
        let samples = self.samples.expect("samples must be set via .samples(...)");
        let mut config = ExecutionConfig::new(samples);
        if let Some(duration) = self.time_budget {
            config = ExecutionConfig::set_time_budget(config, duration);
        }
        if let Some(budget) = self.token_budget {
            config = ExecutionConfig::set_token_budget(config, budget);
        }
        if let Some(pacing) = self.pacing {
            config = ExecutionConfig::set_pacing(config, pacing);
        }

        MeasureExperiment {
            service_contract_id: self
                .service_contract_id
                .expect("service_contract_id must be set via .service_contract_id(...)"),
            factory: self
                .factory
                .expect("service_contract factory must be set via .service_contract(...)"),
            config,
            inputs: self.inputs.expect("inputs must be set via .inputs(...)"),
            trial: self.trial.expect("trial must be set via .trial(...)"),
            experiment_id: self.experiment_id,
            spec_resolver: Some(self.spec_resolver),
            covariate_keys: self.covariate_keys,
            covariate_profile: self.covariate_profile,
            expires_in_days: self.expires_in_days,
        }
    }
}

/// Result of a measure experiment.
#[derive(Debug)]
pub struct MeasureResult {
    execution: ExecutionResult,
    spec: BaselineSpec,
    spec_path: Option<PathBuf>,
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

    struct TestService;

    fn succeeding_trial(_uc: &TestService, _input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    #[test]
    fn produces_baseline_spec() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("test-service")
            .service_contract(|| TestService)
            .samples(100)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .experiment_id("baseline-v1")
            .build()
            .run();

        let spec = result.spec();
        assert_eq!(spec.service_contract_id, "test-service");
        assert_eq!(spec.experiment_id.as_deref(), Some("baseline-v1"));
        assert_eq!(spec.statistics.successes, 100);
        assert_eq!(spec.statistics.failures, 0);
        assert!(spec.requirements.min_pass_rate > 0.9);
        assert!(spec.cost.is_some());
    }

    #[test]
    fn writes_spec_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = vec!["input".to_string()];

        let result = MeasureExperiment::builder()
            .service_contract_id("disk-test")
            .service_contract(|| TestService)
            .samples(50)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .baseline_dir(dir.path())
            .build()
            .run();

        assert!(result.spec_path().is_some());
        assert!(result.spec_path().unwrap().exists());
    }

    #[test]
    fn tracks_failure_distribution() {
        let inputs = vec!["input".to_string()];
        let call_count = std::cell::Cell::new(0u32);
        let result = MeasureExperiment::builder()
            .service_contract_id("mixed-service")
            .service_contract(|| TestService)
            .samples(10)
            .inputs(&inputs)
            .trial(|_uc: &TestService, _input| {
                call_count.set(call_count.get() + 1);
                if call_count.get() % 3 == 0 {
                    TrialOutcome::failure(
                        crate::model::ContractViolation::new("parse", "bad json"),
                        Duration::from_millis(1),
                    )
                } else {
                    TrialOutcome::success(Duration::from_millis(1))
                }
            })
            .build()
            .run();

        let spec = result.spec();
        assert!(spec.statistics.failures > 0);
        assert!(spec.statistics.failure_distribution.is_some());
    }

    #[test]
    fn no_experiment_id_produces_none() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("no-id")
            .service_contract(|| TestService)
            .samples(10)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .build()
            .run();
        assert!(result.spec().experiment_id.is_none());
    }

    #[test]
    fn experiment_id_is_recorded() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("eid-test")
            .service_contract(|| TestService)
            .samples(10)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .experiment_id("v1")
            .build()
            .run();
        assert_eq!(result.spec().experiment_id.as_deref(), Some("v1"));
    }

    #[test]
    fn baseline_dir_writes_to_custom_path() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("custom-dir")
            .service_contract(|| TestService)
            .samples(10)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .baseline_dir(dir.path())
            .build()
            .run();
        assert!(result.spec_path().is_some());
        assert!(result.spec_path().unwrap().exists());
    }

    #[test]
    fn all_successes_has_empty_failure_distribution() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("all-pass")
            .service_contract(|| TestService)
            .samples(20)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .build()
            .run();

        let spec = result.spec();
        assert_eq!(spec.statistics.failures, 0);
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
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("cost-test")
            .service_contract(|| TestService)
            .samples(10)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .build()
            .run();
        let cost = result.spec().cost.as_ref().unwrap();
        assert!(cost.total_time_ms > 0 || cost.avg_time_per_sample_ms == 0);
    }

    #[test]
    fn latency_distribution_captured() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("latency-cap")
            .service_contract(|| TestService)
            .samples(20)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .build()
            .run();
        let latency = result.spec().statistics.latency_distribution.as_ref();
        assert!(latency.is_some());
        assert!(!latency.unwrap().latencies_ms.is_empty());
    }

    #[test]
    fn execution_result_accessible() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("exec-access")
            .service_contract(|| TestService)
            .samples(10)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .build()
            .run();
        assert_eq!(result.execution().summary().successes(), 10);
    }

    #[test]
    fn zero_expires_in_days_omits_expiration_block() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("no-expiry")
            .service_contract(|| TestService)
            .samples(10)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .build()
            .run();
        assert!(result.spec().expiration.is_none());
    }

    #[test]
    fn expires_in_days_populates_expiration_block() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("with-expiry")
            .service_contract(|| TestService)
            .samples(10)
            .inputs(&inputs)
            .trial(succeeding_trial)
            .expires_in_days(30)
            .build()
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

    // --- Builder precondition tests (setter-level validation) ---

    #[test]
    #[should_panic(expected = "samples must be positive")]
    fn rejects_zero_samples() {
        let _ = MeasureExperiment::<TestService>::builder().samples(0);
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn rejects_empty_inputs() {
        let empty: Vec<String> = vec![];
        let _ = MeasureExperiment::<TestService>::builder().inputs(&empty);
    }

    // --- Builder precondition tests (missing-required at build) ---

    #[test]
    #[should_panic(expected = "samples must be set via .samples(")]
    fn build_without_any_required_fields_panics() {
        let _ = MeasureExperiment::<TestService>::builder().build();
    }
}
