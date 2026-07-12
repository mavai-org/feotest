//! Measure experiment: establishing precise empirical baselines.

use std::path::PathBuf;
use std::time::Duration;

use std::collections::BTreeMap;

use crate::controls::{Cost, ExecutionConfig, PacingConfig, TokenRecorder};
use crate::criteria::{CriterionCounts, CriterionTarget};
use crate::experiment::engine::{ContractExecutionResult, ExecutionEngine, SampleEvaluation};
use crate::experiment::judgement::{self, JudgementState, NormativeJudgement};
use crate::service_contract::ServiceContract;
use crate::spec::SpecResolver;
use crate::spec::baseline::{
    BaselineSpec, CriterionStatistics, ExpirationBlock, RequirementsBlock, StatisticsBlock,
};
use crate::spec::common::{
    build_cost_block, build_execution_block, build_failure_distribution,
    build_latency_distribution, build_success_rate_block, iso8601_plus_days, now_iso8601, round4,
    wilson_lower_bound,
};
use crate::spec::namer::CovariateProfile;

type ServiceContractFactory<'a, T> = Box<dyn Fn() -> T + 'a>;

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
/// [`service_contract`](MeasureExperimentBuilder::service_contract). Measure's
/// factory takes no arguments because the experiment measures a single
/// condition; explore and optimize pass a factor into theirs.
///
/// # Examples
///
/// ```no_run
/// use feotest::experiment::MeasureExperiment;
/// # fn run<C>(contract_factory: impl Fn() -> C, inputs: &[String])
/// # where C: feotest::service_contract::ServiceContract<Input = String, Output = String> {
/// let result = MeasureExperiment::builder()
///     .service_contract_id("my-service")
///     .service_contract(contract_factory)
///     .samples(1000)
///     .inputs(inputs)
///     .build()
///     .run();
/// # }
/// ```
// javai-ref: JVI-315MNJX — do not remove (resolves in javai-orchestrator)
pub struct MeasureExperiment<'a, T: ServiceContract> {
    service_contract_id: String,
    factory: ServiceContractFactory<'a, T>,
    config: ExecutionConfig,
    inputs: &'a [T::Input],
    experiment_id: Option<String>,
    spec_resolver: Option<SpecResolver>,
    covariate_keys: Vec<String>,
    covariate_profile: CovariateProfile,
    expires_in_days: u32,
}

impl<'a, T: ServiceContract> MeasureExperiment<'a, T>
where
    T::Output: 'static,
{
    /// Starts a new builder for a measure experiment.
    ///
    /// Required fields (`service_contract_id`, `service_contract` factory,
    /// `samples`, `inputs`) must be set via their corresponding setters
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
    ///
    /// When the contract declares normative criteria
    /// (`Criterion::meeting().pass_rate(..)`), each is judged against its
    /// stipulated threshold using the run's own samples: the judgement is
    /// rendered in the experiment's output and recorded in the baseline
    /// spec, but never affects this method's completion — the judgement
    /// states a relation to the stipulation, and characterising a service
    /// below its bar is a legitimate outcome of a measurement. Use
    /// [`assert_meets`](Self::assert_meets) to make the stipulations
    /// binding instead.
    ///
    /// # Panics
    ///
    /// Panics if a service invocation yields a defect (a transport failure or a
    /// caught panic) — a defect aborts the experiment.
    #[allow(
        clippy::must_use_candidate,
        reason = "callers commonly run for the baseline-persistence side effect alone"
    )]
    pub fn run(self) -> MeasureResult {
        self.execute(false)
    }

    /// Runs the measure experiment and asserts every normative judgement.
    ///
    /// The gating alternative to [`run`](Self::run) — the two are mutually
    /// exclusive terminals on the built experiment. `assert_meets` performs
    /// the same run and the same baseline persistence, then asserts: the
    /// baseline spec is on disk before any failure propagates, so a failed
    /// stipulation never costs the baseline.
    ///
    /// Empirical criteria in the same contract are characterised exactly as
    /// under [`run`](Self::run), never judged.
    ///
    /// # Panics
    ///
    /// - If the contract declares no normative criteria — nothing to assert;
    ///   this is a configuration defect, use [`run`](Self::run). Detected
    ///   before any samples execute.
    /// - If any normative judgement is unsupportable at the run's sample
    ///   count. The cargo harness has no abort channel, so the failure's
    ///   distinct wording — `unsupportable judgement at this sample size`,
    ///   with the feasible minimum stated — is what distinguishes a run
    ///   that could not answer the question from one that answered it
    ///   unfavourably (`normative judgement failed`).
    /// - If any normative judgement failed — the run's evidence did not
    ///   clear a stipulated threshold.
    /// - In every case in which [`run`](Self::run) panics (a defect aborts
    ///   the experiment).
    pub fn assert_meets(self) {
        let result = self.execute(true);
        // The baseline spec is already persisted inside execute; only now
        // may a failure propagate.
        let unsupportable: Vec<&NormativeJudgement> = result
            .judgements()
            .iter()
            .filter(|j| matches!(j.state(), JudgementState::Unsupportable { .. }))
            .collect();
        assert!(
            unsupportable.is_empty(),
            "\n\nunsupportable judgement at this sample size:\n{}\n",
            render_judgement_lines(&unsupportable)
        );
        let failed: Vec<&NormativeJudgement> = result
            .judgements()
            .iter()
            .filter(|j| j.state() == JudgementState::Failed)
            .collect();
        assert!(
            failed.is_empty(),
            "\n\nnormative judgement failed:\n{}\n",
            render_judgement_lines(&failed)
        );
    }

    /// The shared execution path behind [`run`](Self::run) and
    /// [`assert_meets`](Self::assert_meets): sampling, normative judgement,
    /// baseline persistence, and output rendering — in that order.
    ///
    /// # Panics
    ///
    /// Panics if `require_normative` is set and the contract declares no
    /// normative criteria, or if a service invocation yields a defect.
    fn execute(self, require_normative: bool) -> MeasureResult {
        let service_contract = (self.factory)();
        let criteria = service_contract.criteria();
        if require_normative {
            let has_normative = criteria
                .targets()
                .iter()
                .any(|(_, target)| matches!(target, CriterionTarget::NormativeRate(_)));
            assert!(
                has_normative,
                "assert_meets() requires at least one normative criterion \
                 (Criterion::meeting().pass_rate(..)) — a contract with no stipulated \
                 thresholds has nothing to assert; use run()"
            );
        }

        let token_recorder = TokenRecorder::new();
        let result = {
            let cost_recorder = token_recorder.clone();
            let criteria = &criteria;
            ExecutionEngine::run_contract(
                &self.config,
                self.inputs,
                &token_recorder,
                crate::controls::run::current(),
                |input: &T::Input| {
                    let mut cost = Cost::new();
                    let start = std::time::Instant::now();
                    let output = service_contract.invoke(input, &mut cost)?;
                    let elapsed = start.elapsed();
                    cost_recorder.record(cost.tokens_recorded());
                    let expected = service_contract.expected(input);
                    Ok(SampleEvaluation {
                        results: criteria.evaluate(&output, expected.as_ref()),
                        elapsed,
                    })
                },
            )
            .unwrap_or_else(|defect| {
                panic!("\n\nservice invocation aborted the measure experiment: {defect}\n");
            })
        };

        let judgements =
            judgement::judge_normative_criteria(&criteria.targets(), result.criteria_counts());

        let spec = build_spec(
            &self.service_contract_id,
            &self.config,
            &result,
            self.experiment_id.as_deref(),
            self.expires_in_days,
            &judgements,
        );

        let cov_keys: Vec<&str> = self.covariate_keys.iter().map(String::as_str).collect();
        let spec_path = self.spec_resolver.as_ref().and_then(|resolver| {
            resolver
                .write(&spec, &cov_keys, &self.covariate_profile)
                .ok()
        });

        render_output(
            &self.service_contract_id,
            &result,
            &judgements,
            spec_path.as_deref(),
        );

        MeasureResult {
            execution: result,
            spec,
            spec_path,
            judgements,
        }
    }
}

/// Renders the experiment's normative judgements to standard output, each
/// against the criterion's measured characterisation. Silent for contracts
/// with no normative criteria — the characterisation lives in the baseline
/// spec, and there is no judgement to distinguish it from.
fn render_output(
    service_contract_id: &str,
    result: &ContractExecutionResult,
    judgements: &[NormativeJudgement],
    spec_path: Option<&std::path::Path>,
) {
    if judgements.is_empty() {
        return;
    }
    println!(
        "measure \"{service_contract_id}\": {} samples",
        result.summary().samples_executed()
    );
    for judgement in judgements {
        let (pass, total) = result
            .criteria_counts()
            .get(judgement.criterion())
            .map_or((0, 0), |tally| (tally.pass(), tally.total()));
        match judgement.observed_rate() {
            Some(rate) => println!(
                "  criterion \"{}\": observed {rate:.4} ({pass}/{total})",
                judgement.criterion()
            ),
            None => println!(
                "  criterion \"{}\": no samples recorded",
                judgement.criterion()
            ),
        }
        println!("  {judgement}");
    }
    if let Some(path) = spec_path {
        println!("  baseline written: {}", path.display());
    }
}

/// Renders one line per judgement, for assertion messages.
fn render_judgement_lines(judgements: &[&NormativeJudgement]) -> String {
    judgements
        .iter()
        .map(|j| format!("  criterion \"{}\": {j}", j.criterion()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_spec(
    service_contract_id: &str,
    config: &ExecutionConfig,
    result: &ContractExecutionResult,
    experiment_id: Option<&str>,
    expires_in_days: u32,
    judgements: &[NormativeJudgement],
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
            // Each criterion's own measured rate, so an empirical criterion can
            // later derive its target from its own baseline.
            per_criterion: build_per_criterion(result, judgements),
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

/// Builds the per-criterion baseline statistics from the run's tallies.
///
/// Emitted for multi-criterion contracts — so an empirical criterion testing
/// against the baseline can later derive its target from its own rate — and
/// whenever a normative judgement was rendered, since the judgement marker is
/// recorded per criterion. A single-criterion contract with no normative
/// criterion emits no block: the aggregate figures already describe it, and
/// an empirical criterion testing against such a baseline falls back to the
/// aggregate rate.
fn build_per_criterion(
    result: &ContractExecutionResult,
    judgements: &[NormativeJudgement],
) -> Option<BTreeMap<String, CriterionStatistics>> {
    let per = result.criteria_counts().per_criterion();
    if per.len() <= 1 && judgements.is_empty() {
        return None;
    }
    let map = per
        .iter()
        .map(|counts: &CriterionCounts| {
            let stats = CriterionStatistics {
                success_rate: build_success_rate_block(counts.pass(), counts.total()),
                successes: counts.pass(),
                failures: counts.fail(),
                failure_distribution: (!counts.failure_distribution().is_empty())
                    .then(|| counts.failure_distribution().clone()),
                normative_judgement: judgements
                    .iter()
                    .find(|j| j.criterion() == counts.criterion())
                    .map(NormativeJudgement::to_spec_block),
            };
            (counts.criterion().to_string(), stats)
        })
        .collect();
    Some(map)
}

/// Fluent builder for [`MeasureExperiment`].
///
/// Required fields — `service_contract_id`, `service_contract` (factory),
/// `samples`, and `inputs` — must be set before [`build`](Self::build)
/// is called. Missing any of them produces a panic naming the field
/// and the setter to call.
///
/// Optional fields carry documented defaults. Setters that validate a
/// single value (e.g., positive sample count, non-empty inputs) panic
/// at the setter rather than deferring to `build`.
pub struct MeasureExperimentBuilder<'a, T: ServiceContract> {
    service_contract_id: Option<String>,
    factory: Option<ServiceContractFactory<'a, T>>,
    samples: Option<u32>,
    inputs: Option<&'a [T::Input]>,
    experiment_id: Option<String>,
    spec_resolver: SpecResolver,
    covariate_keys: Vec<String>,
    covariate_profile: CovariateProfile,
    expires_in_days: u32,
    time_budget: Option<Duration>,
    token_budget: Option<u64>,
    pacing: Option<PacingConfig>,
}

impl<T: ServiceContract> Default for MeasureExperimentBuilder<'_, T> {
    fn default() -> Self {
        Self {
            service_contract_id: None,
            factory: None,
            samples: None,
            inputs: None,
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

impl<'a, T: ServiceContract> MeasureExperimentBuilder<'a, T> {
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
    /// experiment, invoked and judged on every sample, and dropped when the
    /// run completes.
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

    /// Sets the inputs the contract is invoked against.
    ///
    /// # Panics
    ///
    /// Panics if `inputs` is empty.
    #[must_use]
    pub fn inputs(mut self, inputs: &'a [T::Input]) -> Self {
        assert!(!inputs.is_empty(), "inputs must not be empty");
        self.inputs = Some(inputs);
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
    /// [`ContractTest::fail_on_expired_baseline`](crate::ptest::ContractTest::fail_on_expired_baseline).
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
    /// `samples`, `inputs`) is missing.
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
    execution: ContractExecutionResult,
    spec: BaselineSpec,
    spec_path: Option<PathBuf>,
    judgements: Vec<NormativeJudgement>,
}

impl MeasureResult {
    /// The execution result.
    #[must_use]
    pub const fn execution(&self) -> &ContractExecutionResult {
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

    /// The normative judgements rendered for this run, one per normative
    /// criterion the contract declared, in declaration order. Empty when the
    /// contract declares no normative criteria — empirical criteria are
    /// never judged at experiment time.
    #[must_use]
    pub fn judgements(&self) -> &[NormativeJudgement] {
        &self.judgements
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::criteria::{Criteria, Criterion};
    use crate::model::{ContractViolation, Defect, Outcome};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A contract whose single criterion always passes.
    struct TestService;

    impl ServiceContract for TestService {
        type Input = String;
        type Output = String;

        fn id(&self) -> &'static str {
            "test-service"
        }

        fn invoke(&self, input: &String, _cost: &mut Cost) -> Result<String, Defect> {
            Ok(input.clone())
        }

        fn criteria(&self) -> Criteria<String> {
            Criteria::of([Criterion::meeting()
                .pass_rate(0.5)
                .name("content")
                .satisfies("content", |_: &String| -> Outcome { Ok(()) })
                .build()])
        }
    }

    /// A contract whose response — and so its single criterion — fails on every
    /// third invocation, giving a deterministic failure mix.
    struct MixedService {
        calls: AtomicU32,
    }

    impl ServiceContract for MixedService {
        type Input = String;
        type Output = String;

        fn id(&self) -> &'static str {
            "mixed-service"
        }

        fn invoke(&self, _input: &String, _cost: &mut Cost) -> Result<String, Defect> {
            let n = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            Ok(if n % 3 == 0 { "fail" } else { "ok" }.to_string())
        }

        fn criteria(&self) -> Criteria<String> {
            Criteria::of([Criterion::meeting()
                .pass_rate(0.5)
                .name("content")
                .satisfies("parse", |r: &String| -> Outcome {
                    if r == "fail" {
                        Err(ContractViolation::new("parse", "bad json"))
                    } else {
                        Ok(())
                    }
                })
                .build()])
        }
    }

    #[test]
    fn produces_baseline_spec() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("test-service")
            .service_contract(|| TestService)
            .samples(100)
            .inputs(&inputs)
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
            .baseline_dir(dir.path())
            .build()
            .run();

        assert!(result.spec_path().is_some());
        assert!(result.spec_path().unwrap().exists());
    }

    #[test]
    fn tracks_failure_distribution() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("mixed-service")
            .service_contract(|| MixedService {
                calls: AtomicU32::new(0),
            })
            .samples(10)
            .inputs(&inputs)
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

    // --- Normative judgement at experiment time ---

    use crate::spec::baseline::NormativeJudgementState;

    /// A contract with one normative criterion at a configurable stipulated
    /// rate, judging a service that fails every third invocation
    /// (observed rate ≈ 2/3).
    struct StipulatedService {
        stipulated: f64,
        calls: AtomicU32,
    }

    impl StipulatedService {
        fn with_stipulation(stipulated: f64) -> Self {
            Self {
                stipulated,
                calls: AtomicU32::new(0),
            }
        }
    }

    impl ServiceContract for StipulatedService {
        type Input = String;
        type Output = String;

        fn id(&self) -> &'static str {
            "stipulated-service"
        }

        fn invoke(&self, _input: &String, _cost: &mut Cost) -> Result<String, Defect> {
            let n = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            Ok(if n % 3 == 0 { "fail" } else { "ok" }.to_string())
        }

        fn criteria(&self) -> Criteria<String> {
            let stipulated = self.stipulated;
            Criteria::of([Criterion::meeting()
                .pass_rate(stipulated)
                .name("content")
                .satisfies("parse", |r: &String| -> Outcome {
                    if r == "fail" {
                        Err(ContractViolation::new("parse", "bad json"))
                    } else {
                        Ok(())
                    }
                })
                .build()])
        }
    }

    /// A contract mixing a normative criterion with an empirical one.
    struct MixedTargetsService;

    impl ServiceContract for MixedTargetsService {
        type Input = String;
        type Output = String;

        fn id(&self) -> &'static str {
            "mixed-targets"
        }

        fn invoke(&self, input: &String, _cost: &mut Cost) -> Result<String, Defect> {
            Ok(input.clone())
        }

        fn criteria(&self) -> Criteria<String> {
            Criteria::of([
                Criterion::meeting()
                    .pass_rate(0.5)
                    .name("stipulated")
                    .satisfies("always", |_: &String| -> Outcome { Ok(()) })
                    .build(),
                Criterion::empirical()
                    .pass_rate()
                    .name("measured")
                    .satisfies("always", |_: &String| -> Outcome { Ok(()) })
                    .build(),
            ])
        }
    }

    /// A contract with only an empirical criterion. When `reachable` is
    /// false, `invoke` panics — used to prove that `assert_meets` detects
    /// the configuration defect before any sample executes.
    struct EmpiricalOnlyService {
        reachable: bool,
    }

    impl ServiceContract for EmpiricalOnlyService {
        type Input = String;
        type Output = String;

        fn id(&self) -> &'static str {
            "empirical-only"
        }

        fn invoke(&self, input: &String, _cost: &mut Cost) -> Result<String, Defect> {
            assert!(
                self.reachable,
                "a configuration defect must be detected before sampling"
            );
            Ok(input.clone())
        }

        fn criteria(&self) -> Criteria<String> {
            Criteria::of([Criterion::empirical()
                .pass_rate()
                .name("measured")
                .satisfies("always", |_: &String| -> Outcome { Ok(()) })
                .build()])
        }
    }

    /// The number of baseline spec files written into `dir`.
    fn written_spec_count(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "yaml")
            })
            .count()
    }

    fn measure_stipulated(stipulated: f64, samples: u32) -> MeasureResult {
        let inputs = vec!["input".to_string()];
        MeasureExperiment::builder()
            .service_contract_id("stipulated-service")
            .service_contract(move || StipulatedService::with_stipulation(stipulated))
            .samples(samples)
            .inputs(&inputs)
            .build()
            .run()
    }

    #[test]
    fn clearing_evidence_reports_met_and_records_the_marker() {
        // Observed ≈ 2/3 against a stipulation of 0.5: the Wilson lower
        // bound at 300 samples clears it.
        let result = measure_stipulated(0.5, 300);

        assert_eq!(result.judgements().len(), 1);
        assert_eq!(result.judgements()[0].state(), JudgementState::Met);

        let marker = result.spec().statistics.per_criterion.as_ref().unwrap()["content"]
            .normative_judgement
            .clone()
            .unwrap();
        assert_eq!(marker.state, NormativeJudgementState::Met);
        assert!((marker.stipulated_threshold - 0.5).abs() < 1e-12);
        assert!((marker.confidence - 0.95).abs() < 1e-12);
    }

    #[test]
    fn short_evidence_reports_failed_and_run_still_completes() {
        // Observed ≈ 2/3 against a stipulation of 0.9: failed — and run()
        // completes regardless, with the baseline spec intact.
        let result = measure_stipulated(0.9, 300);

        assert_eq!(result.judgements()[0].state(), JudgementState::Failed);
        assert_eq!(result.spec().statistics.successes, 200);

        let marker = result.spec().statistics.per_criterion.as_ref().unwrap()["content"]
            .normative_judgement
            .clone()
            .unwrap();
        assert_eq!(marker.state, NormativeJudgementState::Failed);
        assert!(marker.feasible_minimum_samples.is_none());
    }

    #[test]
    fn undersized_run_reports_unsupportable_with_feasible_minimum() {
        // 10 samples cannot support a 0.99 stipulation at 95% confidence,
        // even with a perfect observation. No panic, no silent omission.
        let result = measure_stipulated(0.99, 10);

        let JudgementState::Unsupportable {
            feasible_minimum_samples,
        } = result.judgements()[0].state()
        else {
            panic!("expected unsupportable");
        };
        assert!(feasible_minimum_samples > 10);

        let marker = result.spec().statistics.per_criterion.as_ref().unwrap()["content"]
            .normative_judgement
            .clone()
            .unwrap();
        assert_eq!(marker.state, NormativeJudgementState::Unsupportable);
        assert_eq!(
            marker.feasible_minimum_samples,
            Some(feasible_minimum_samples)
        );
    }

    #[test]
    fn mixed_contract_judges_only_the_normative_criterion() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("mixed-targets")
            .service_contract(|| MixedTargetsService)
            .samples(50)
            .inputs(&inputs)
            .build()
            .run();

        assert_eq!(result.judgements().len(), 1);
        assert_eq!(result.judgements()[0].criterion(), "stipulated");

        let per_criterion = result.spec().statistics.per_criterion.as_ref().unwrap();
        assert!(per_criterion["stipulated"].normative_judgement.is_some());
        assert!(per_criterion["measured"].normative_judgement.is_none());
    }

    #[test]
    fn contract_without_normative_criteria_yields_no_judgements() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::builder()
            .service_contract_id("empirical-only")
            .service_contract(|| EmpiricalOnlyService { reachable: true })
            .samples(10)
            .inputs(&inputs)
            .build()
            .run();
        assert!(result.judgements().is_empty());
        // A single empirical criterion also emits no per-criterion block —
        // the aggregate figures already describe it.
        assert!(result.spec().statistics.per_criterion.is_none());
    }

    #[test]
    fn assert_meets_completes_when_every_judgement_is_met() {
        let inputs = vec!["input".to_string()];
        let dir = tempfile::tempdir().unwrap();
        MeasureExperiment::builder()
            .service_contract_id("stipulated-service")
            .service_contract(|| StipulatedService::with_stipulation(0.5))
            .samples(300)
            .inputs(&inputs)
            .baseline_dir(dir.path())
            .build()
            .assert_meets();
        assert_eq!(written_spec_count(dir.path()), 1);
    }

    #[test]
    fn assert_meets_fails_the_harness_after_persisting_the_baseline() {
        let inputs = vec!["input".to_string()];
        let dir = tempfile::tempdir().unwrap();

        let path = dir.path().to_path_buf();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            MeasureExperiment::builder()
                .service_contract_id("stipulated-service")
                .service_contract(|| StipulatedService::with_stipulation(0.9))
                .samples(300)
                .inputs(&inputs)
                .baseline_dir(path)
                .build()
                .assert_meets();
        }));

        let panic_payload = outcome.expect_err("a failed judgement must fail the harness");
        let message = panic_payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(message.contains("normative judgement failed"));
        assert!(message.contains("stipulated threshold 0.9"));
        // Persistence strictly precedes assertion: the baseline spec is on
        // disk even though the stipulation failed.
        assert_eq!(written_spec_count(dir.path()), 1);
    }

    #[test]
    fn assert_meets_reports_unsupportable_with_the_feasible_minimum() {
        let inputs = vec!["input".to_string()];
        let dir = tempfile::tempdir().unwrap();

        let path = dir.path().to_path_buf();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            MeasureExperiment::builder()
                .service_contract_id("stipulated-service")
                .service_contract(|| StipulatedService::with_stipulation(0.99))
                .samples(10)
                .inputs(&inputs)
                .baseline_dir(path)
                .build()
                .assert_meets();
        }));

        let panic_payload = outcome.expect_err("an unsupportable judgement must abort the case");
        let message = panic_payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(message.contains("unsupportable judgement at this sample size"));
        assert!(!message.contains("normative judgement failed"));
        assert!(message.contains("feasible minimum"));
        // The baseline spec is still persisted before the abort.
        assert_eq!(written_spec_count(dir.path()), 1);
    }

    #[test]
    #[should_panic(expected = "requires at least one normative criterion")]
    fn assert_meets_on_a_contract_without_normative_criteria_is_a_defect() {
        let inputs = vec!["input".to_string()];
        MeasureExperiment::builder()
            .service_contract_id("empirical-only")
            .service_contract(|| EmpiricalOnlyService { reachable: false })
            .samples(10)
            .inputs(&inputs)
            .build()
            .assert_meets();
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
