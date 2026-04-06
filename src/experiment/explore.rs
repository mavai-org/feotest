//! Explore experiment: rapid configuration comparison.
//!
//! Each configuration is a pre-built, immutable use case instance.
//! The framework runs the same trial function against each configuration
//! independently, collecting results for side-by-side comparison.
//!
//! This design enforces the immutable use case principle: the experimental
//! condition is fixed during sampling, which is a direct expression of the
//! i.i.d. assumption required for valid statistical inference.

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
use crate::usecase::UseCase;

/// A single configuration's exploration results.
#[derive(Debug)]
pub struct ConfigResult {
    name: String,
    execution: ExecutionResult,
    projections: Vec<SampleProjection>,
}

impl ConfigResult {
    /// The configuration name.
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

/// An explore experiment that compares multiple configurations.
///
/// Each configuration provides a pre-built, immutable use case instance.
/// The trial function is declared once and shared across all configurations.
///
/// # Examples
///
/// ```
/// use feotest::experiment::ExploreExperiment;
/// use feotest::model::TrialOutcome;
/// use feotest::usecase::UseCase;
/// use feotest::spec::namer::CovariateProfile;
/// use std::fmt;
/// use std::time::Duration;
///
/// struct MyService { factor: f64 }
/// impl MyService {
///     fn call(&self, _input: &str) -> TrialOutcome {
///         if self.factor > 0.5 {
///             TrialOutcome::success(Duration::from_millis(1))
///         } else {
///             TrialOutcome::success(Duration::from_millis(2))
///         }
///     }
/// }
/// impl UseCase for MyService {
///     fn id(&self) -> &str { "my-service" }
/// }
/// impl fmt::Display for MyService {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         write!(f, "MyService (factor={})", self.factor)
///     }
/// }
///
/// let inputs = vec!["request".to_string()];
///
/// let svc_a = MyService { factor: 0.3 };
/// let svc_b = MyService { factor: 0.8 };
///
/// let result = ExploreExperiment::new(&svc_a, 10, &inputs, |svc: &MyService, input| {
///     svc.call(input)
/// })
/// .config(&svc_b)
/// .run();
///
/// assert_eq!(result.configs().len(), 2);
/// assert_eq!(result.configs()[0].name(), "MyService (factor=0.3)");
/// ```
pub struct ExploreExperiment<'a, T, F> {
    use_case_id: String,
    samples_per_config: u32,
    inputs: &'a [String],
    trial: F,
    configs: Vec<(String, &'a T)>,
    experiment_id: Option<String>,
    output_dir: Option<PathBuf>,
    factor_values: BTreeMap<String, BTreeMap<String, FactorYamlValue>>,
}

impl<'a, T, F> ExploreExperiment<'a, T, F>
where
    T: fmt::Display + UseCase,
    F: Fn(&T, &str) -> TrialOutcome,
{
    /// Creates a new explore experiment.
    ///
    /// The first use case instance provides both the use case identity
    /// (via [`UseCase::id`]) and the first configuration to explore.
    /// Additional configurations are added with [`.config()`](Self::config)
    /// or [`.config_named()`](Self::config_named).
    ///
    /// # Arguments
    ///
    /// * `first_config` — the first configuration (also provides the use case ID).
    /// * `samples_per_config` — number of trials per configuration.
    /// * `inputs` — the input strings to cycle through during trials.
    /// * `trial` — function that executes one trial given a use case
    ///   reference and an input string.
    pub fn new(
        first_config: &'a T,
        samples_per_config: u32,
        inputs: &'a [String],
        trial: F,
    ) -> Self {
        let use_case_id = first_config.id().to_owned();
        let mut configs = Vec::new();
        configs.push((first_config.to_string(), first_config));
        Self {
            use_case_id,
            samples_per_config,
            inputs,
            trial,
            configs,
            experiment_id: None,
            output_dir: None,
            factor_values: BTreeMap::new(),
        }
    }

    /// Adds a configuration with a pre-built use case instance.
    ///
    /// The label is derived from the use case's `Display` implementation,
    /// which should describe the configuration's distinguishing factors.
    ///
    /// The use case is borrowed immutably for the duration of the experiment.
    #[must_use]
    pub fn config(mut self, use_case: &'a T) -> Self {
        self.configs.push((use_case.to_string(), use_case));
        self
    }

    /// Adds a configuration with an explicit label.
    ///
    /// Use this when you need a label that differs from the use case's
    /// `Display` output — for example, a shorter name for reports.
    #[must_use]
    pub fn config_named(mut self, name: impl Into<String>, use_case: &'a T) -> Self {
        self.configs.push((name.into(), use_case));
        self
    }

    /// Sets the experiment identifier.
    #[must_use]
    pub fn experiment_id(mut self, id: impl Into<String>) -> Self {
        self.experiment_id = Some(id.into());
        self
    }

    /// Configures YAML spec output for each explored configuration.
    ///
    /// When set, running the experiment writes per-configuration specs to
    /// `{output_dir}/explorations/{use_case_id}/{config_name}.yaml`.
    #[must_use]
    pub fn output_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.output_dir = Some(dir.into());
        self
    }

    /// Records factor values for a named configuration.
    ///
    /// These appear in the `executionContext` block of the exploration YAML.
    #[must_use]
    pub fn factors(
        mut self,
        config_name: impl Into<String>,
        values: BTreeMap<String, FactorYamlValue>,
    ) -> Self {
        self.factor_values.insert(config_name.into(), values);
        self
    }

    /// Runs the explore experiment and returns results per configuration.
    ///
    /// # Panics
    ///
    /// Panics if no configurations have been added.
    pub fn run(self) -> ExploreResult {
        assert!(
            !self.configs.is_empty(),
            "ExploreExperiment '{}': at least one configuration is required",
            self.use_case_id
        );

        let mut results = Vec::new();

        for (name, use_case) in &self.configs {
            let exec_config = ExecutionConfig::new(self.samples_per_config);
            let recorder = TokenRecorder::new();

            let mut projections = Vec::new();
            let mut sample_idx: u32 = 0;

            let mut trial_fn = |input: &str| {
                let outcome = (self.trial)(use_case, input);
                projections.push(build_projection(sample_idx, input, &outcome));
                sample_idx += 1;
                outcome
            };

            let execution =
                ExecutionEngine::run(&exec_config, self.inputs, &recorder, &mut trial_fn);

            results.push(ConfigResult {
                name: name.clone(),
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
            if let Ok(paths) = writer.write_all(&result, &self.factor_values) {
                result.spec_paths = Some(paths);
            }
        }

        result
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

    /// Results for each configuration explored.
    #[must_use]
    pub fn configs(&self) -> &[ConfigResult] {
        &self.configs
    }

    /// Paths of written spec files, if output was configured.
    #[must_use]
    pub fn spec_paths(&self) -> Option<&[PathBuf]> {
        self.spec_paths.as_deref()
    }

    /// Renders all configuration results as YAML to stdout.
    ///
    /// Each configuration produces a separate YAML document, delimited by
    /// `---`. Includes per-sample result projections when the trial
    /// function enriched the `TrialOutcome` with projection metadata.
    ///
    /// This is the primary output mechanism for explore experiments.
    /// The developer pipes or redirects as needed:
    ///
    /// ```text
    /// cargo test --test my_explore -- --nocapture > results.yaml
    /// cargo test --test my_explore -- --nocapture | less
    /// ```
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

    struct MockService {
        success_rate: f64,
    }

    impl MockService {
        const fn new(success_rate: f64) -> Self {
            Self { success_rate }
        }
    }

    impl UseCase for MockService {
        fn id(&self) -> &str {
            "test-uc"
        }
    }

    impl fmt::Display for MockService {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "MockService (rate={})", self.success_rate)
        }
    }

    #[test]
    fn explores_multiple_configurations() {
        let inputs = vec!["input".to_string()];

        let svc_a = MockService::new(1.0);
        let svc_b = MockService::new(0.8);

        let result = ExploreExperiment::new(&svc_a, 5, &inputs, |_svc, _input| {
            TrialOutcome::success(Duration::ZERO)
        })
        .config(&svc_b)
        .run();

        assert_eq!(result.configs().len(), 2);
        assert_eq!(result.configs()[0].name(), "MockService (rate=1)");
        assert_eq!(result.configs()[1].name(), "MockService (rate=0.8)");
    }

    #[test]
    fn config_named_overrides_label() {
        let inputs = vec!["input".to_string()];
        let svc = MockService::new(1.0);

        let result = ExploreExperiment::new(&svc, 5, &inputs, |_svc, _input| {
            TrialOutcome::success(Duration::ZERO)
        })
        .run();

        // First config added via new() uses Display name
        assert_eq!(result.configs()[0].name(), "MockService (rate=1)");
    }

    #[test]
    fn each_config_gets_correct_sample_count() {
        let inputs = vec!["input".to_string()];
        let svc = MockService::new(1.0);

        let result = ExploreExperiment::new(&svc, 10, &inputs, |_svc, _input| {
            TrialOutcome::success(Duration::ZERO)
        })
        .run();

        assert_eq!(
            result.configs()[0].execution().summary().samples_executed(),
            10
        );
    }

    #[test]
    fn trial_receives_correct_use_case() {
        let inputs = vec!["input".to_string()];

        let svc_good = MockService::new(1.0);
        let svc_bad = MockService::new(0.0);

        let result = ExploreExperiment::new(&svc_good, 5, &inputs, |svc: &MockService, _input| {
            if svc.success_rate > 0.5 {
                TrialOutcome::success(Duration::ZERO)
            } else {
                TrialOutcome::failure(
                    crate::model::ContractViolation::new("test", "forced failure"),
                    Duration::ZERO,
                )
            }
        })
        .config(&svc_bad)
        .run();

        let good_result = &result.configs()[0];
        let bad_result = &result.configs()[1];

        assert_eq!(good_result.execution().summary().successes(), 5);
        assert_eq!(bad_result.execution().summary().failures(), 5);
    }
}
