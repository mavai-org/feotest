//! Explore experiment: rapid configuration comparison.
//!
//! Each configuration is a pre-built, immutable use case instance.
//! The framework runs the same trial function against each configuration
//! independently, collecting results for side-by-side comparison.
//!
//! This design enforces the immutable use case principle: the experimental
//! condition is fixed during sampling, which is a direct expression of the
//! i.i.d. assumption required for valid statistical inference.

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::engine::{ExecutionEngine, ExecutionResult};
use crate::model::TrialOutcome;

/// A single configuration's exploration results.
#[derive(Debug)]
pub struct ConfigResult {
    name: String,
    execution: ExecutionResult,
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
///
/// let inputs = vec!["request".to_string()];
///
/// let svc_a = MyService { factor: 0.3 };
/// let svc_b = MyService { factor: 0.8 };
///
/// let result = ExploreExperiment::new("MyService", 10, &inputs, |svc: &MyService, input| {
///     svc.call(input)
/// })
/// .config("factor=0.3", &svc_a)
/// .config("factor=0.8", &svc_b)
/// .run();
///
/// assert_eq!(result.configs().len(), 2);
/// ```
pub struct ExploreExperiment<'a, T, F> {
    use_case_id: String,
    samples_per_config: u32,
    inputs: &'a [String],
    trial: F,
    configs: Vec<(String, &'a T)>,
    experiment_id: Option<String>,
}

impl<'a, T, F> ExploreExperiment<'a, T, F>
where
    F: Fn(&T, &str) -> TrialOutcome,
{
    /// Creates a new explore experiment.
    ///
    /// # Arguments
    ///
    /// * `use_case_id` — identifies the use case.
    /// * `samples_per_config` — number of trials per configuration.
    /// * `inputs` — the input strings to cycle through during trials.
    /// * `trial` — function that executes one trial given a use case
    ///   reference and an input string.
    pub fn new(
        use_case_id: impl Into<String>,
        samples_per_config: u32,
        inputs: &'a [String],
        trial: F,
    ) -> Self {
        Self {
            use_case_id: use_case_id.into(),
            samples_per_config,
            inputs,
            trial,
            configs: Vec::new(),
            experiment_id: None,
        }
    }

    /// Adds a named configuration with a pre-built use case instance.
    ///
    /// The use case is borrowed immutably for the duration of the experiment.
    /// It must not be mutated between calling `.config()` and `.run()`.
    #[must_use]
    pub fn config(mut self, name: impl Into<String>, use_case: &'a T) -> Self {
        self.configs.push((name.into(), use_case));
        self
    }

    /// Sets the experiment identifier.
    #[must_use]
    pub fn experiment_id(mut self, id: impl Into<String>) -> Self {
        self.experiment_id = Some(id.into());
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

            let mut trial_fn = |input: &str| (self.trial)(use_case, input);

            let execution =
                ExecutionEngine::run(&exec_config, self.inputs, &recorder, &mut trial_fn);

            results.push(ConfigResult {
                name: name.clone(),
                execution,
            });
        }

        ExploreResult {
            use_case_id: self.use_case_id,
            experiment_id: self.experiment_id,
            configs: results,
        }
    }
}

/// Result of an explore experiment.
#[derive(Debug)]
pub struct ExploreResult {
    use_case_id: String,
    experiment_id: Option<String>,
    configs: Vec<ConfigResult>,
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

    #[test]
    fn explores_multiple_configurations() {
        let inputs = vec!["input".to_string()];

        let svc_a = MockService::new(1.0);
        let svc_b = MockService::new(1.0);

        let result = ExploreExperiment::new("test-uc", 5, &inputs, |_svc, _input| {
            TrialOutcome::success(Duration::ZERO)
        })
        .config("config-a", &svc_a)
        .config("config-b", &svc_b)
        .run();

        assert_eq!(result.configs().len(), 2);
        assert_eq!(result.configs()[0].name(), "config-a");
        assert_eq!(result.configs()[1].name(), "config-b");
    }

    #[test]
    fn each_config_gets_correct_sample_count() {
        let inputs = vec!["input".to_string()];
        let svc = MockService::new(1.0);

        let result = ExploreExperiment::new("test-uc", 10, &inputs, |_svc, _input| {
            TrialOutcome::success(Duration::ZERO)
        })
        .config("single", &svc)
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

        let result = ExploreExperiment::new("test-uc", 5, &inputs, |svc: &MockService, _input| {
            if svc.success_rate > 0.5 {
                TrialOutcome::success(Duration::ZERO)
            } else {
                TrialOutcome::failure(
                    crate::model::ContractViolation::new("test", "forced failure"),
                    Duration::ZERO,
                )
            }
        })
        .config("good", &svc_good)
        .config("bad", &svc_bad)
        .run();

        let good_result = &result.configs()[0];
        let bad_result = &result.configs()[1];

        assert_eq!(good_result.execution().summary().successes(), 5);
        assert_eq!(bad_result.execution().summary().failures(), 5);
    }
}
