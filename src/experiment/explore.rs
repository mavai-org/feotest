//! Explore experiment: rapid configuration comparison.
//!
//! An explore experiment compares configurations of a single use case.
//! Each configuration is described by a **factor** (typically a struct
//! carrying the values that distinguish this configuration from the
//! others). The framework walks a list of factors, calling a
//! user-supplied factory to produce one use case instance per factor,
//! and runs a fixed number of trials against each instance.
//!
//! This design enforces the immutable use case principle: the
//! experimental condition (the factor) is fixed during sampling, which
//! is a direct expression of the i.i.d. assumption required for valid
//! statistical inference. It also makes the "one use case, many
//! configurations" constraint structural — there is one factory, so
//! every instance is by construction a variant of the same use case.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::engine::{ExecutionEngine, ExecutionResult};
use crate::model::TrialOutcome;
use crate::spec::baseline::ExecutionBlock;
use crate::spec::common::{build_cost_block, build_failure_distribution, now_iso8601, round4};
use crate::spec::explore::{
    ExplorationSpec, ExplorationStatisticsBlock, ExploreSpecWriter, FactorYamlValue,
};
use crate::spec::projection::{SampleProjection, build_projection, format_projections};

type UseCaseFactory<'a, F, T> = Box<dyn Fn(&F) -> T + 'a>;
type TrialClosure<'a, T> = Box<dyn Fn(&T, &str) -> TrialOutcome + 'a>;

/// A single configuration's exploration results.
#[derive(Debug)]
pub struct ConfigResult {
    name: String,
    execution: ExecutionResult,
    projections: Vec<SampleProjection>,
}

impl ConfigResult {
    /// The configuration name, derived from the factor's `Display` impl.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The execution result for this configuration.
    #[must_use]
    pub const fn execution(&self) -> &ExecutionResult {
        &self.execution
    }

    /// Per-sample result projections.
    #[must_use]
    pub fn projections(&self) -> &[SampleProjection] {
        &self.projections
    }
}

/// An explore experiment that compares multiple configurations of a
/// single use case.
///
/// Construct via [`ExploreExperiment::builder`]; there is no public
/// constructor.
///
/// # Examples
///
/// ```
/// use feotest::experiment::ExploreExperiment;
/// use feotest::model::TrialOutcome;
/// use std::fmt;
/// use std::time::Duration;
///
/// // The factor: the values that distinguish one configuration from
/// // another. `Display` yields the configuration's name in reports.
/// #[derive(Clone)]
/// struct BasketFactors {
///     model: &'static str,
///     temperature: f64,
/// }
/// impl fmt::Display for BasketFactors {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         write!(f, "{}_t{}", self.model, self.temperature)
///     }
/// }
///
/// // The use case: what the factory produces.
/// struct ShoppingBasket { model: &'static str, temperature: f64 }
/// impl ShoppingBasket {
///     fn new(model: &'static str, temperature: f64) -> Self {
///         Self { model, temperature }
///     }
///     fn search(&self, _input: &str) -> TrialOutcome {
///         TrialOutcome::success(Duration::from_millis(1))
///     }
/// }
///
/// let factors = vec![
///     BasketFactors { model: "gpt-4",    temperature: 0.0 },
///     BasketFactors { model: "gpt-4",    temperature: 0.7 },
///     BasketFactors { model: "claude-3", temperature: 0.0 },
/// ];
/// let inputs = vec!["request".to_string()];
///
/// let result = ExploreExperiment::builder()
///     .use_case_id("shopping-basket")
///     .factors(factors)
///     .use_case(|f: &BasketFactors| ShoppingBasket::new(f.model, f.temperature))
///     .samples_per_config(10)
///     .inputs(&inputs)
///     .trial(|uc: &ShoppingBasket, input| uc.search(input))
///     .build()
///     .run();
///
/// assert_eq!(result.configs().len(), 3);
/// assert_eq!(result.configs()[0].name(), "gpt-4_t0");
/// ```
pub struct ExploreExperiment<'a, F, T> {
    use_case_id: String,
    factors: Vec<F>,
    factory: UseCaseFactory<'a, F, T>,
    samples_per_config: u32,
    inputs: &'a [String],
    trial: TrialClosure<'a, T>,
    experiment_id: Option<String>,
    output_dir: Option<PathBuf>,
}

impl<'a, F, T> ExploreExperiment<'a, F, T>
where
    F: fmt::Display,
{
    /// Starts a new builder for an explore experiment.
    ///
    /// Required fields (`use_case_id`, `factors`, `use_case`,
    /// `samples_per_config`, `inputs`, `trial`) must be set via their
    /// corresponding setters before
    /// [`build`](ExploreExperimentBuilder::build) is called.
    #[must_use]
    pub fn builder() -> ExploreExperimentBuilder<'a, F, T> {
        ExploreExperimentBuilder::default()
    }

    /// Runs the explore experiment and returns results per configuration.
    pub fn run(self) -> ExploreResult {
        let mut results = Vec::new();

        for factor in &self.factors {
            let use_case = (self.factory)(factor);
            let name = factor.to_string();

            let exec_config = ExecutionConfig::new(self.samples_per_config);
            let recorder = TokenRecorder::new();

            let mut projections = Vec::new();
            let mut sample_idx: u32 = 0;

            let mut trial_fn = |input: &str| {
                let outcome = (self.trial)(&use_case, input);
                projections.push(build_projection(sample_idx, input, &outcome));
                sample_idx += 1;
                outcome
            };

            let execution = ExecutionEngine::run(
                &exec_config,
                self.inputs,
                &recorder,
                crate::controls::run::current(),
                &mut trial_fn,
            );

            results.push(ConfigResult {
                name,
                execution,
                projections,
            });
        }

        let mut result = ExploreResult {
            use_case_id: self.use_case_id,
            experiment_id: self.experiment_id,
            configs: results,
            spec_paths: None,
        };

        if let Some(ref dir) = self.output_dir {
            let writer = ExploreSpecWriter::new(dir);
            let empty_factor_values: BTreeMap<String, BTreeMap<String, FactorYamlValue>> =
                BTreeMap::new();
            if let Ok(paths) = writer.write_all(&result, &empty_factor_values) {
                result.spec_paths = Some(paths);
            }
        }

        result
    }
}

/// Fluent builder for [`ExploreExperiment`].
///
/// Required fields — `use_case_id`, `factors`, `use_case` (factory),
/// `samples_per_config`, `inputs`, and `trial` — must be set before
/// [`build`](Self::build) is called. Missing any of them produces a
/// panic naming the field and the setter to call.
///
/// Setters that validate a single value (e.g. a non-empty factor list,
/// positive sample count, non-empty inputs) panic at the setter rather
/// than deferring to `build`.
pub struct ExploreExperimentBuilder<'a, F, T> {
    use_case_id: Option<String>,
    factors: Vec<F>,
    factory: Option<UseCaseFactory<'a, F, T>>,
    samples_per_config: Option<u32>,
    inputs: Option<&'a [String]>,
    trial: Option<TrialClosure<'a, T>>,
    experiment_id: Option<String>,
    output_dir: Option<PathBuf>,
}

impl<F, T> Default for ExploreExperimentBuilder<'_, F, T> {
    fn default() -> Self {
        Self {
            use_case_id: None,
            factors: Vec::new(),
            factory: None,
            samples_per_config: None,
            inputs: None,
            trial: None,
            experiment_id: None,
            output_dir: None,
        }
    }
}

impl<'a, F, T> ExploreExperimentBuilder<'a, F, T>
where
    F: fmt::Display,
{
    // --- required fields ---

    /// Sets the use case identifier.
    ///
    /// This appears in the spec YAML and in the output directory layout.
    /// All configurations in the experiment share this id — the point of
    /// an explore experiment is to compare variants of one use case.
    #[must_use]
    pub fn use_case_id(mut self, id: impl Into<String>) -> Self {
        self.use_case_id = Some(id.into());
        self
    }

    /// Sets the list of factors to explore.
    ///
    /// Each factor is one configuration of the use case. The factory
    /// set via [`use_case`](Self::use_case) is called once per factor
    /// to produce the corresponding use case instance.
    ///
    /// The factor's `Display` implementation provides the configuration
    /// name used in reports and output filenames.
    ///
    /// # Panics
    ///
    /// Panics if `factors` is empty.
    #[must_use]
    pub fn factors(mut self, factors: Vec<F>) -> Self {
        assert!(!factors.is_empty(), "factors must not be empty");
        self.factors = factors;
        self
    }

    /// Sets the use case factory.
    ///
    /// Given a factor, the factory produces one use case instance. The
    /// framework calls the factory once per factor, runs
    /// `samples_per_config` trials against the resulting instance, then
    /// drops it.
    #[must_use]
    pub fn use_case(mut self, factory: impl Fn(&F) -> T + 'a) -> Self {
        self.factory = Some(Box::new(factory));
        self
    }

    /// Sets the number of samples to run per configuration.
    ///
    /// # Panics
    ///
    /// Panics if `samples` is zero.
    #[must_use]
    pub fn samples_per_config(mut self, samples: u32) -> Self {
        assert!(samples > 0, "samples_per_config must be positive, got 0");
        self.samples_per_config = Some(samples);
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
    /// The closure receives a reference to the current configuration's
    /// use case instance and an input string, and returns a
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

    /// Configures YAML spec output for each explored configuration.
    ///
    /// When set, running the experiment writes per-configuration specs
    /// to `{output_dir}/{use_case_id}/{config_name}.yaml`, where
    /// `config_name` is the factor's `Display` output. Default: no
    /// output files are written.
    #[must_use]
    pub fn output_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.output_dir = Some(dir.into());
        self
    }

    /// Builds the [`ExploreExperiment`].
    ///
    /// # Panics
    ///
    /// Panics if any required field is missing: `use_case_id`,
    /// `factors`, `use_case` (factory), `samples_per_config`, `inputs`,
    /// or `trial`.
    #[must_use]
    pub fn build(self) -> ExploreExperiment<'a, F, T> {
        ExploreExperiment {
            use_case_id: self
                .use_case_id
                .expect("use_case_id must be set via .use_case_id(...)"),
            factors: {
                assert!(
                    !self.factors.is_empty(),
                    "factors must be set via .factors(...)"
                );
                self.factors
            },
            factory: self
                .factory
                .expect("use_case factory must be set via .use_case(...)"),
            samples_per_config: self
                .samples_per_config
                .expect("samples_per_config must be set via .samples_per_config(...)"),
            inputs: self.inputs.expect("inputs must be set via .inputs(...)"),
            trial: self.trial.expect("trial must be set via .trial(...)"),
            experiment_id: self.experiment_id,
            output_dir: self.output_dir,
        }
    }
}

/// Result of an explore experiment.
#[derive(Debug)]
pub struct ExploreResult {
    use_case_id: String,
    experiment_id: Option<String>,
    configs: Vec<ConfigResult>,
    spec_paths: Option<Vec<PathBuf>>,
}

impl ExploreResult {
    /// The use case identifier.
    #[must_use]
    pub fn use_case_id(&self) -> &str {
        &self.use_case_id
    }

    /// The experiment identifier.
    #[must_use]
    pub fn experiment_id(&self) -> Option<&str> {
        self.experiment_id.as_deref()
    }

    /// Results for each configuration explored, in factor order.
    #[must_use]
    pub fn configs(&self) -> &[ConfigResult] {
        &self.configs
    }

    /// Paths of written spec files, if output was configured.
    #[must_use]
    pub fn spec_paths(&self) -> Option<&[PathBuf]> {
        self.spec_paths.as_deref()
    }

    /// Renders all configuration results as YAML.
    ///
    /// Each configuration produces a separate YAML document, delimited
    /// by `---`. Includes per-sample result projections when the trial
    /// function enriched the `TrialOutcome` with projection metadata.
    pub fn to_yaml(&self) -> String {
        let timestamp = now_iso8601();
        let mut out = String::new();

        for (i, config) in self.configs.iter().enumerate() {
            if i > 0 {
                let _ = writeln!(out, "---");
            }

            let summary = config.execution().summary();
            let agg = config.execution().aggregate();

            let spec = ExplorationSpec {
                schema_version: "feotest-spec-1".to_owned(),
                use_case_id: self.use_case_id.clone(),
                generated_at: timestamp.clone(),
                experiment_id: self.experiment_id.clone(),
                execution_context: BTreeMap::new(),
                execution: ExecutionBlock {
                    samples_planned: summary.samples_planned(),
                    samples_executed: summary.samples_executed(),
                    termination_reason: Some(summary.termination().reason().to_string()),
                },
                statistics: ExplorationStatisticsBlock {
                    observed: round4(summary.observed_pass_rate()),
                    successes: summary.successes(),
                    failures: summary.failures(),
                    failure_distribution: build_failure_distribution(agg),
                },
                cost: Some(build_cost_block(summary.cost())),
            };

            if let Ok(yaml) = spec.to_yaml() {
                let _ = write!(out, "{yaml}");
            }

            let projection_yaml = format_projections(config.projections());
            if !projection_yaml.is_empty() {
                let _ = write!(out, "{projection_yaml}");
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Clone)]
    struct RateFactor {
        success_rate: f64,
    }
    impl fmt::Display for RateFactor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "rate={}", self.success_rate)
        }
    }

    struct MockService {
        success_rate: f64,
    }
    impl MockService {
        const fn from_factor(factor: &RateFactor) -> Self {
            Self {
                success_rate: factor.success_rate,
            }
        }
    }

    #[test]
    fn explores_multiple_configurations() {
        let inputs = vec!["input".to_string()];
        let factors = vec![
            RateFactor { success_rate: 1.0 },
            RateFactor { success_rate: 0.8 },
        ];

        let result = ExploreExperiment::builder()
            .use_case_id("test-uc")
            .factors(factors)
            .use_case(MockService::from_factor)
            .samples_per_config(5)
            .inputs(&inputs)
            .trial(|_svc: &MockService, _input| TrialOutcome::success(Duration::ZERO))
            .build()
            .run();

        assert_eq!(result.configs().len(), 2);
        assert_eq!(result.configs()[0].name(), "rate=1");
        assert_eq!(result.configs()[1].name(), "rate=0.8");
    }

    #[test]
    fn each_config_gets_correct_sample_count() {
        let inputs = vec!["input".to_string()];
        let factors = vec![RateFactor { success_rate: 1.0 }];

        let result = ExploreExperiment::builder()
            .use_case_id("test-uc")
            .factors(factors)
            .use_case(MockService::from_factor)
            .samples_per_config(10)
            .inputs(&inputs)
            .trial(|_svc: &MockService, _input| TrialOutcome::success(Duration::ZERO))
            .build()
            .run();

        assert_eq!(
            result.configs()[0].execution().summary().samples_executed(),
            10
        );
    }

    #[test]
    fn factory_receives_current_factor() {
        let inputs = vec!["input".to_string()];
        let factors = vec![
            RateFactor { success_rate: 1.0 },
            RateFactor { success_rate: 0.0 },
        ];

        let result = ExploreExperiment::builder()
            .use_case_id("test-uc")
            .factors(factors)
            .use_case(MockService::from_factor)
            .samples_per_config(5)
            .inputs(&inputs)
            .trial(|svc: &MockService, _input| {
                if svc.success_rate > 0.5 {
                    TrialOutcome::success(Duration::ZERO)
                } else {
                    TrialOutcome::failure(
                        crate::model::ContractViolation::new("test", "forced failure"),
                        Duration::ZERO,
                    )
                }
            })
            .build()
            .run();

        assert_eq!(result.configs()[0].execution().summary().successes(), 5);
        assert_eq!(result.configs()[1].execution().summary().failures(), 5);
    }

    // --- Builder precondition tests (setter-level validation) ---

    #[test]
    #[should_panic(expected = "factors must not be empty")]
    fn rejects_empty_factors() {
        let empty: Vec<RateFactor> = vec![];
        let _ = ExploreExperiment::<RateFactor, MockService>::builder().factors(empty);
    }

    #[test]
    #[should_panic(expected = "samples_per_config must be positive")]
    fn rejects_zero_samples_per_config() {
        let _ = ExploreExperiment::<RateFactor, MockService>::builder().samples_per_config(0);
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn rejects_empty_inputs() {
        let empty: Vec<String> = vec![];
        let _ = ExploreExperiment::<RateFactor, MockService>::builder().inputs(&empty);
    }

    // --- Builder precondition tests (missing-required at build) ---

    #[test]
    #[should_panic(expected = "use_case_id must be set")]
    fn build_without_any_required_fields_panics() {
        let _ = ExploreExperiment::<RateFactor, MockService>::builder().build();
    }
}
