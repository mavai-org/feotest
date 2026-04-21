//! Optimize experiment: iterative factor tuning.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::engine::{ExecutionEngine, ExecutionResult};
use crate::model::TrialOutcome;
use crate::spec::optimization::{OptimizationSpec, OptimizeSpecWriter};
use crate::usecase::{FactorValue, UseCase};

/// A scoring function that evaluates an iteration's results.
pub trait Scorer: Send + Sync {
    /// Scores the result of a single iteration.
    ///
    /// Higher scores are better for `Maximize`, lower for `Minimize`.
    fn score(&self, result: &ExecutionResult) -> f64;
}

/// Generates new factor values based on optimisation history.
pub trait FactorMutator: Send + Sync {
    /// Produces the next factor value given the current value and history.
    fn mutate(&self, current: &FactorValue, history: &[IterationRecord]) -> FactorValue;
}

/// Whether to maximise or minimise the score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Objective {
    /// Seek the highest score.
    Maximize,
    /// Seek the lowest score.
    Minimize,
}

/// Why an optimisation run stopped iterating.
///
/// The set of variants is the same across all javai frameworks so that
/// optimisation YAML output is cross-project comparable. Not every variant
/// is reachable in every runtime: feotest currently terminates only on
/// `MaxIterations` or `NoImprovement`; the others become reachable as
/// budget and threshold controls mature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationReason {
    /// The configured maximum iteration count was reached.
    MaxIterations,
    /// The no-improvement window elapsed without a new best score.
    NoImprovement,
    /// A wall-clock time budget expired.
    TimeBudget,
    /// A token budget was exhausted.
    TokenBudget,
    /// A user-supplied score threshold was reached.
    ScoreThresholdReached,
}

impl TerminationReason {
    /// The canonical string used in YAML output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxIterations => "MAX_ITERATIONS",
            Self::NoImprovement => "NO_IMPROVEMENT",
            Self::TimeBudget => "TIME_BUDGET",
            Self::TokenBudget => "TOKEN_BUDGET",
            Self::ScoreThresholdReached => "SCORE_THRESHOLD_REACHED",
        }
    }
}

impl fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Record of a single optimisation iteration.
#[derive(Debug, Clone)]
pub struct IterationRecord {
    iteration: u32,
    factor_value: FactorValue,
    score: f64,
    successes: u32,
    failures: u32,
}

impl IterationRecord {
    /// The iteration number (0-indexed).
    #[must_use]
    pub const fn iteration(&self) -> u32 {
        self.iteration
    }

    /// The factor value used in this iteration.
    #[must_use]
    pub const fn factor_value(&self) -> &FactorValue {
        &self.factor_value
    }

    /// The score achieved.
    #[must_use]
    pub const fn score(&self) -> f64 {
        self.score
    }

    /// Number of successes in this iteration.
    #[must_use]
    pub const fn successes(&self) -> u32 {
        self.successes
    }

    /// Number of failures in this iteration.
    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }
}

/// An optimize experiment that iteratively tunes a single control factor.
pub struct OptimizeExperiment<'a, F> {
    use_case_id: String,
    control_factor: String,
    initial_value: FactorValue,
    scorer: Box<dyn Scorer>,
    mutator: Box<dyn FactorMutator>,
    objective: Objective,
    samples_per_iteration: u32,
    max_iterations: u32,
    no_improvement_window: u32,
    inputs: &'a [String],
    trial: F,
    apply_factor: Box<dyn FnMut(&FactorValue)>,
    experiment_id: Option<String>,
}

impl<'a, F> OptimizeExperiment<'a, F>
where
    F: FnMut(&str) -> TrialOutcome,
{
    /// Creates a new optimize experiment.
    ///
    /// # Parameters
    ///
    /// - `use_case`: The use case (provides identity).
    /// - `control_factor`: Name of the factor being optimised.
    /// - `initial_value`: Starting value for the control factor.
    /// - `scorer`: Scoring function.
    /// - `mutator`: Factor mutation strategy.
    /// - `inputs`: Input values for trials.
    /// - `trial`: Trial closure.
    /// - `apply_factor`: Closure that applies a factor value to the use case.
    /// # Panics
    ///
    /// Panics if `inputs` is empty.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        use_case: &dyn UseCase,
        control_factor: impl Into<String>,
        initial_value: FactorValue,
        scorer: impl Scorer + 'static,
        mutator: impl FactorMutator + 'static,
        inputs: &'a [String],
        trial: F,
        apply_factor: impl FnMut(&FactorValue) + 'static,
    ) -> Self {
        assert!(!inputs.is_empty(), "inputs must not be empty");
        Self {
            use_case_id: use_case.id().to_owned(),
            control_factor: control_factor.into(),
            initial_value,
            scorer: Box::new(scorer),
            mutator: Box::new(mutator),
            objective: Objective::Maximize,
            samples_per_iteration: 20,
            max_iterations: 20,
            no_improvement_window: 5,
            inputs,
            trial,
            apply_factor: Box::new(apply_factor),
            experiment_id: None,
        }
    }

    /// Sets the optimisation objective.
    #[must_use]
    pub const fn with_objective(mut self, objective: Objective) -> Self {
        self.objective = objective;
        self
    }

    /// Sets samples per iteration.
    ///
    /// # Panics
    ///
    /// Panics if `samples` is zero.
    #[must_use]
    pub fn with_samples_per_iteration(mut self, samples: u32) -> Self {
        assert!(samples > 0, "samples_per_iteration must be positive, got 0");
        self.samples_per_iteration = samples;
        self
    }

    /// Sets maximum iterations.
    ///
    /// # Panics
    ///
    /// Panics if `max` is zero.
    #[must_use]
    pub fn with_max_iterations(mut self, max: u32) -> Self {
        assert!(max > 0, "max_iterations must be positive, got 0");
        self.max_iterations = max;
        self
    }

    /// Sets the no-improvement window for early termination.
    ///
    /// # Panics
    ///
    /// Panics if `window` is zero.
    #[must_use]
    pub fn with_no_improvement_window(mut self, window: u32) -> Self {
        assert!(window > 0, "no_improvement_window must be positive, got 0");
        self.no_improvement_window = window;
        self
    }

    /// Sets the experiment identifier.
    #[must_use]
    pub fn with_experiment_id(mut self, id: impl Into<String>) -> Self {
        self.experiment_id = Some(id.into());
        self
    }

    /// Runs the optimisation and returns the result.
    pub fn run(mut self) -> OptimizeResult {
        let mut history: Vec<IterationRecord> = Vec::new();
        let mut current_value = self.initial_value.clone();
        let mut best_score: Option<f64> = None;
        let mut best_iteration: Option<u32> = None;
        let mut no_improvement_count = 0u32;
        let mut termination_reason = TerminationReason::MaxIterations;

        for iteration in 0..self.max_iterations {
            // Apply the current factor value
            (self.apply_factor)(&current_value);

            // Run samples for this iteration
            let config = ExecutionConfig::new(self.samples_per_iteration);
            let recorder = TokenRecorder::new();
            let result =
                ExecutionEngine::run(&config, self.inputs, &recorder, None, &mut self.trial);

            let score = self.scorer.score(&result);

            let record = IterationRecord {
                iteration,
                factor_value: current_value.clone(),
                score,
                successes: result.summary().successes(),
                failures: result.summary().failures(),
            };
            history.push(record);

            // Check for improvement
            let improved = match (best_score, self.objective) {
                (None, _) => true,
                (Some(best), Objective::Maximize) => score > best,
                (Some(best), Objective::Minimize) => score < best,
            };

            if improved {
                best_score = Some(score);
                best_iteration = Some(iteration);
                no_improvement_count = 0;
            } else {
                no_improvement_count += 1;
            }

            // Check plateau termination
            if no_improvement_count >= self.no_improvement_window {
                termination_reason = TerminationReason::NoImprovement;
                break;
            }

            // Mutate for next iteration (unless this is the last)
            if iteration + 1 < self.max_iterations {
                current_value = self.mutator.mutate(&current_value, &history);
            }
        }

        OptimizeResult {
            use_case_id: self.use_case_id,
            control_factor: self.control_factor,
            objective: self.objective,
            experiment_id: self.experiment_id,
            history,
            best_iteration,
            best_score,
            termination_reason,
        }
    }
}

/// Result of an optimize experiment.
#[derive(Debug)]
pub struct OptimizeResult {
    use_case_id: String,
    control_factor: String,
    objective: Objective,
    experiment_id: Option<String>,
    history: Vec<IterationRecord>,
    best_iteration: Option<u32>,
    best_score: Option<f64>,
    termination_reason: TerminationReason,
}

impl OptimizeResult {
    /// The use case identifier.
    #[must_use]
    pub fn use_case_id(&self) -> &str {
        &self.use_case_id
    }

    /// The control factor that was optimised.
    #[must_use]
    pub fn control_factor(&self) -> &str {
        &self.control_factor
    }

    /// The optimisation objective.
    #[must_use]
    pub const fn objective(&self) -> Objective {
        self.objective
    }

    /// The experiment identifier.
    #[must_use]
    pub fn experiment_id(&self) -> Option<&str> {
        self.experiment_id.as_deref()
    }

    /// Full iteration history.
    #[must_use]
    pub fn history(&self) -> &[IterationRecord] {
        &self.history
    }

    /// The iteration number with the best score.
    #[must_use]
    pub const fn best_iteration(&self) -> Option<u32> {
        self.best_iteration
    }

    /// The best score achieved.
    #[must_use]
    pub const fn best_score(&self) -> Option<f64> {
        self.best_score
    }

    /// The factor value that produced the best score.
    #[must_use]
    pub fn best_factor_value(&self) -> Option<&FactorValue> {
        self.best_iteration.and_then(|idx| {
            self.history
                .iter()
                .find(|r| r.iteration == idx)
                .map(|r| &r.factor_value)
        })
    }

    /// Why optimisation stopped iterating.
    #[must_use]
    pub const fn termination_reason(&self) -> TerminationReason {
        self.termination_reason
    }

    /// Builds the canonical [`OptimizationSpec`] for this result.
    #[must_use]
    pub fn to_spec(&self) -> OptimizationSpec {
        OptimizationSpec::from_result(self)
    }

    /// Serialises the result to the canonical YAML schema.
    ///
    /// # Errors
    ///
    /// Returns an error if YAML serialisation fails.
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        self.to_spec().to_yaml()
    }

    /// Writes the optimization YAML artefact under the given output root.
    ///
    /// The final path is `{root}/{use_case_id}/{experiment_id}.yaml`. The
    /// default output root is `target/feotest/optimizations/` — see
    /// [`write_to_default`](Self::write_to_default).
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation, YAML serialisation, or
    /// file writing fails.
    pub fn write_to(&self, root: impl AsRef<Path>) -> Result<PathBuf, std::io::Error> {
        OptimizeSpecWriter::new(root.as_ref().to_path_buf()).write(self)
    }

    /// Writes the artefact to the framework default output root.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation, YAML serialisation, or
    /// file writing fails.
    pub fn write_to_default(&self) -> Result<PathBuf, std::io::Error> {
        OptimizeSpecWriter::with_default_root().write(self)
    }
}

impl fmt::Display for OptimizeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let objective_label = match self.objective {
            Objective::Maximize => "maximize",
            Objective::Minimize => "minimize",
        };

        writeln!(
            f,
            "OptimizeResult: {} ({objective_label} '{}')",
            self.use_case_id, self.control_factor,
        )?;

        if let Some(id) = &self.experiment_id {
            writeln!(f, "  experiment: {id}")?;
        }

        writeln!(f, "  iterations: {}", self.history.len())?;

        if let (Some(best_iter), Some(best_score)) = (self.best_iteration, self.best_score) {
            writeln!(f, "  best: iteration {best_iter}, score {best_score:.4}")?;
            if let Some(value) = self.best_factor_value() {
                writeln!(f, "  best value: {value}")?;
            }
        } else {
            writeln!(f, "  best: none")?;
        }

        if !self.history.is_empty() {
            writeln!(f, "  history:")?;
            for record in &self.history {
                writeln!(
                    f,
                    "    [{:>2}] {} → score {:.4} ({} ok, {} fail)",
                    record.iteration,
                    record.factor_value,
                    record.score,
                    record.successes,
                    record.failures,
                )?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct SuccessRateScorer;
    impl Scorer for SuccessRateScorer {
        fn score(&self, result: &ExecutionResult) -> f64 {
            result.summary().observed_pass_rate()
        }
    }

    struct IncrementMutator;
    impl FactorMutator for IncrementMutator {
        fn mutate(&self, current: &FactorValue, _history: &[IterationRecord]) -> FactorValue {
            match current {
                FactorValue::Float(v) => FactorValue::Float(v + 0.1),
                other => other.clone(),
            }
        }
    }

    struct TestUc(&'static str);
    impl UseCase for TestUc {
        fn id(&self) -> &str {
            self.0
        }
    }

    #[test]
    fn runs_optimization_iterations() {
        let uc = TestUc("test-uc");
        let inputs = vec!["input".to_string()];
        let current_temp = Arc::new(Mutex::new(0.5_f64));

        let temp_for_apply = Arc::clone(&current_temp);
        let temp_for_trial = Arc::clone(&current_temp);

        let result = OptimizeExperiment::new(
            &uc,
            "temperature",
            FactorValue::Float(0.5),
            SuccessRateScorer,
            IncrementMutator,
            &inputs,
            move |_input| {
                let temp = *temp_for_trial.lock().unwrap();
                // Lower temp = higher success rate
                if temp < 0.7 {
                    TrialOutcome::success(Duration::ZERO)
                } else {
                    TrialOutcome::failure(
                        crate::model::ContractViolation::new("quality", "low"),
                        Duration::ZERO,
                    )
                }
            },
            move |value| {
                if let FactorValue::Float(v) = value {
                    *temp_for_apply.lock().unwrap() = *v;
                }
            },
        )
        .with_max_iterations(5)
        .with_samples_per_iteration(10)
        .with_no_improvement_window(3)
        .with_experiment_id("temp-tune")
        .run();

        assert!(!result.history().is_empty());
        assert!(result.best_score().is_some());
        assert!(result.best_factor_value().is_some());
        assert_eq!(result.control_factor(), "temperature");
    }

    #[test]
    fn display_shows_summary_and_history() {
        let inputs = vec!["input".to_string()];

        let uc = TestUc("display-test");
        let result = OptimizeExperiment::new(
            &uc,
            "temperature",
            FactorValue::Float(0.5),
            SuccessRateScorer,
            IncrementMutator,
            &inputs,
            |_input| TrialOutcome::success(Duration::ZERO),
            |_value| {},
        )
        .with_max_iterations(3)
        .with_samples_per_iteration(5)
        .with_no_improvement_window(10)
        .with_experiment_id("display-exp")
        .run();

        let output = result.to_string();
        assert!(output.contains("display-test"));
        assert!(output.contains("temperature"));
        assert!(output.contains("maximize"));
        assert!(output.contains("display-exp"));
        assert!(output.contains("best:"));
        assert!(output.contains("history:"));
    }

    #[test]
    fn stops_on_plateau() {
        let inputs = vec!["input".to_string()];

        let uc = TestUc("test-uc");
        let result = OptimizeExperiment::new(
            &uc,
            "factor",
            FactorValue::Float(1.0),
            SuccessRateScorer,
            IncrementMutator,
            &inputs,
            |_input| TrialOutcome::success(Duration::ZERO),
            |_value| {},
        )
        .with_max_iterations(20)
        .with_no_improvement_window(3)
        .with_samples_per_iteration(5)
        .run();

        // All iterations score 1.0, so after first + 3 no-improvement, should stop at 4
        assert!(result.history().len() <= 4);
    }

    #[test]
    fn minimize_objective() {
        let inputs = vec!["input".to_string()];
        let uc = TestUc("minimize-test");
        let call_count = Arc::new(Mutex::new(0u32));
        let count_for_trial = Arc::clone(&call_count);

        let result = OptimizeExperiment::new(
            &uc,
            "factor",
            FactorValue::Float(1.0),
            SuccessRateScorer,
            IncrementMutator,
            &inputs,
            move |_input| {
                let mut c = count_for_trial.lock().unwrap();
                *c += 1;
                TrialOutcome::success(Duration::ZERO)
            },
            |_value| {},
        )
        .with_objective(Objective::Minimize)
        .with_max_iterations(3)
        .with_samples_per_iteration(5)
        .run();

        assert_eq!(result.objective(), Objective::Minimize);
        assert!(result.best_score().is_some());
    }

    #[test]
    fn display_minimize_label() {
        let inputs = vec!["input".to_string()];
        let uc = TestUc("min-label");

        let result = OptimizeExperiment::new(
            &uc,
            "cost",
            FactorValue::Float(1.0),
            SuccessRateScorer,
            IncrementMutator,
            &inputs,
            |_| TrialOutcome::success(Duration::ZERO),
            |_| {},
        )
        .with_objective(Objective::Minimize)
        .with_max_iterations(2)
        .with_samples_per_iteration(5)
        .run();

        let output = result.to_string();
        assert!(output.contains("minimize"));
    }

    #[test]
    fn single_iteration_produces_best() {
        let inputs = vec!["input".to_string()];
        let uc = TestUc("single-iter");

        let result = OptimizeExperiment::new(
            &uc,
            "factor",
            FactorValue::Float(1.0),
            SuccessRateScorer,
            IncrementMutator,
            &inputs,
            |_| TrialOutcome::success(Duration::ZERO),
            |_| {},
        )
        .with_max_iterations(1)
        .with_samples_per_iteration(5)
        .run();

        assert_eq!(result.history().len(), 1);
        assert_eq!(result.best_iteration(), Some(0));
    }

    #[test]
    fn iteration_record_accessors() {
        let inputs = vec!["input".to_string()];
        let uc = TestUc("accessors");

        let result = OptimizeExperiment::new(
            &uc,
            "factor",
            FactorValue::Float(1.0),
            SuccessRateScorer,
            IncrementMutator,
            &inputs,
            |_| TrialOutcome::success(Duration::ZERO),
            |_| {},
        )
        .with_max_iterations(1)
        .with_samples_per_iteration(5)
        .run();

        let record = &result.history()[0];
        assert_eq!(record.iteration(), 0);
        assert_eq!(record.successes(), 5);
        assert_eq!(record.failures(), 0);
        assert!((record.score() - 1.0).abs() < 1e-10);
        assert!(matches!(record.factor_value(), FactorValue::Float(_)));
    }

    #[test]
    fn result_accessors() {
        let inputs = vec!["input".to_string()];
        let uc = TestUc("result-acc");

        let result = OptimizeExperiment::new(
            &uc,
            "factor",
            FactorValue::Float(1.0),
            SuccessRateScorer,
            IncrementMutator,
            &inputs,
            |_| TrialOutcome::success(Duration::ZERO),
            |_| {},
        )
        .with_max_iterations(2)
        .with_samples_per_iteration(5)
        .with_experiment_id("exp-123")
        .run();

        assert_eq!(result.use_case_id(), "result-acc");
        assert_eq!(result.control_factor(), "factor");
        assert_eq!(result.experiment_id(), Some("exp-123"));
        assert_eq!(result.objective(), Objective::Maximize);
    }

    // --- Precondition tests ---

    fn dummy_experiment(
        inputs: &[String],
    ) -> OptimizeExperiment<'_, impl FnMut(&str) -> TrialOutcome> {
        let uc = TestUc("precondition-test");
        OptimizeExperiment::new(
            &uc,
            "factor",
            FactorValue::Float(0.5),
            SuccessRateScorer,
            IncrementMutator,
            inputs,
            |_| TrialOutcome::success(Duration::ZERO),
            |_| {},
        )
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn rejects_empty_inputs() {
        let inputs: Vec<String> = vec![];
        dummy_experiment(&inputs);
    }

    #[test]
    #[should_panic(expected = "samples_per_iteration must be positive")]
    fn rejects_zero_samples_per_iteration() {
        let inputs = vec!["input".to_string()];
        dummy_experiment(&inputs).with_samples_per_iteration(0);
    }

    #[test]
    #[should_panic(expected = "max_iterations must be positive")]
    fn rejects_zero_max_iterations() {
        let inputs = vec!["input".to_string()];
        dummy_experiment(&inputs).with_max_iterations(0);
    }

    #[test]
    #[should_panic(expected = "no_improvement_window must be positive")]
    fn rejects_zero_no_improvement_window() {
        let inputs = vec!["input".to_string()];
        dummy_experiment(&inputs).with_no_improvement_window(0);
    }
}
