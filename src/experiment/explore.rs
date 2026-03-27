//! Explore experiment: rapid configuration comparison.

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::engine::{ExecutionEngine, ExecutionResult};
use crate::model::TrialOutcome;

/// A named configuration to explore.
pub struct ExploreConfig {
    name: String,
    setup: Box<dyn FnMut()>,
}

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
/// Each configuration is executed independently with the same inputs
/// and trial closure. Results are collected per-configuration for
/// side-by-side comparison.
pub struct ExploreExperiment<'a, F> {
    use_case_id: String,
    samples_per_config: u32,
    inputs: &'a [String],
    trial: F,
    configs: Vec<ExploreConfig>,
    experiment_id: Option<String>,
    warmup: u32,
}

impl<'a, F> ExploreExperiment<'a, F>
where
    F: FnMut(&str) -> TrialOutcome,
{
    /// Creates a new explore experiment.
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
            warmup: 0,
        }
    }

    /// Adds a named configuration with a setup closure.
    ///
    /// The setup closure is called before running trials for this configuration.
    /// Use it to set factors on the use case.
    #[must_use]
    pub fn config(mut self, name: impl Into<String>, setup: impl FnMut() + 'static) -> Self {
        self.configs.push(ExploreConfig {
            name: name.into(),
            setup: Box::new(setup),
        });
        self
    }

    /// Sets the experiment identifier.
    #[must_use]
    pub fn with_experiment_id(mut self, id: impl Into<String>) -> Self {
        self.experiment_id = Some(id.into());
        self
    }

    /// Sets the warmup count applied before each configuration.
    #[must_use]
    pub const fn with_warmup(mut self, warmup: u32) -> Self {
        self.warmup = warmup;
        self
    }

    /// Runs the explore experiment and returns results per configuration.
    pub fn run(mut self) -> ExploreResult {
        let mut results = Vec::new();

        for mut cfg in self.configs.drain(..) {
            // Run setup for this configuration
            (cfg.setup)();

            let exec_config =
                ExecutionConfig::new(self.samples_per_config).with_warmup(self.warmup);
            let recorder = TokenRecorder::new();

            let execution =
                ExecutionEngine::run(&exec_config, self.inputs, &recorder, &mut self.trial);

            results.push(ConfigResult {
                name: cfg.name,
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    #[test]
    fn explores_multiple_configurations() {
        let inputs = vec!["input".to_string()];
        let config_counter = Arc::new(AtomicU32::new(0));

        let c1 = Arc::clone(&config_counter);
        let c2 = Arc::clone(&config_counter);

        let result = ExploreExperiment::new("test-uc", 5, &inputs, |_input| {
            TrialOutcome::success(Duration::ZERO)
        })
        .config("config-a", move || {
            c1.fetch_add(1, Ordering::SeqCst);
        })
        .config("config-b", move || {
            c2.fetch_add(1, Ordering::SeqCst);
        })
        .run();

        assert_eq!(result.configs().len(), 2);
        assert_eq!(result.configs()[0].name(), "config-a");
        assert_eq!(result.configs()[1].name(), "config-b");
        assert_eq!(config_counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn each_config_gets_correct_sample_count() {
        let inputs = vec!["input".to_string()];

        let result = ExploreExperiment::new("test-uc", 10, &inputs, |_input| {
            TrialOutcome::success(Duration::ZERO)
        })
        .config("single", || {})
        .run();

        assert_eq!(
            result.configs()[0].execution().summary().samples_executed(),
            10
        );
    }
}
