//! Optimize experiment: iterative factor tuning.
//!
//! An optimize experiment tunes a single factor by recycling it through
//! a feedback loop: each iteration runs samples against a use case
//! instance built from the current factor, scores the result, records
//! the outcome, and hands control to a mutator that produces the next
//! factor from the history. The loop stops when an iteration cap is
//! reached or the best score stops improving for a configured number of
//! iterations.
//!
//! The API shape mirrors [`super::ExploreExperiment`]: a `factor` is a
//! user-defined type, a `service_contract(factory)` builds an instance from a
//! factor, and a `trial(|uc, input| …)` closure runs against the
//! instance. The only structural difference is how factors are
//! supplied — optimize takes a single `initial_factor` plus a
//! [`FactorMutator`] that drives subsequent factors from history;
//! explore takes them all upfront as a `Vec<F>`.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::engine::{ExecutionEngine, ExecutionResult};
use crate::model::TrialOutcome;
use crate::spec::optimization::{OptimizationSpec, OptimizeSpecWriter};

type ServiceContractFactory<'a, F, T> = Box<dyn Fn(&F) -> T + 'a>;
type TrialClosure<'a, T> = Box<dyn Fn(&T, &str) -> TrialOutcome + 'a>;

/// A scoring function that evaluates an iteration's results.
pub trait Scorer: Send + Sync {
    /// Scores the result of a single iteration.
    ///
    /// Higher scores are better for [`Objective::Maximize`]; lower for
    /// [`Objective::Minimize`].
    fn score(&self, result: &ExecutionResult) -> f64;
}

/// Generates the next factor value from the current one and the
/// iteration history.
pub trait FactorMutator<F>: Send + Sync {
    /// Produces the next factor value given the current value and the
    /// history of prior iterations.
    fn mutate(&self, current: &F, history: &[IterationRecord<F>]) -> F;
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
/// optimisation YAML output is cross-project comparable. Not every
/// variant is reachable in every runtime: feotest currently terminates
/// only on `MaxIterations` or `NoImprovement`; the others become
/// reachable as budget and threshold controls mature.
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
pub struct IterationRecord<F> {
    iteration: u32,
    factor: F,
    score: f64,
    successes: u32,
    failures: u32,
}

impl<F> IterationRecord<F> {
    /// The iteration number (0-indexed).
    #[must_use]
    pub const fn iteration(&self) -> u32 {
        self.iteration
    }

    /// The factor used in this iteration.
    #[must_use]
    pub const fn factor(&self) -> &F {
        &self.factor
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

/// An optimize experiment that iteratively tunes a single factor.
///
/// Construct via [`OptimizeExperiment::builder`]; there is no public
/// constructor.
///
/// # Examples
///
/// ```no_run
/// use feotest::experiment::{
///     ExecutionResult, FactorMutator, IterationRecord, Objective,
///     OptimizeExperiment, Scorer,
/// };
/// use feotest::model::TrialOutcome;
/// use serde::Serialize;
///
/// // Factor type: what varies between iterations.
/// #[derive(Clone, Serialize)]
/// struct Temperature(f64);
///
/// // Use case type: what the factory produces from a factor.
/// struct MyService { temperature: f64 }
/// impl MyService {
///     fn call(&self, _input: &str) -> TrialOutcome {
///         TrialOutcome::success(std::time::Duration::ZERO)
///     }
/// }
///
/// struct PassRateScorer;
/// impl Scorer for PassRateScorer {
///     fn score(&self, result: &ExecutionResult) -> f64 {
///         result.summary().observed_pass_rate()
///     }
/// }
///
/// struct StepMutator;
/// impl FactorMutator<Temperature> for StepMutator {
///     fn mutate(
///         &self,
///         current: &Temperature,
///         _history: &[IterationRecord<Temperature>],
///     ) -> Temperature {
///         Temperature(current.0 + 0.1)
///     }
/// }
///
/// let inputs = vec!["request".to_string()];
///
/// let _ = OptimizeExperiment::builder()
///     .service_contract_id("my-service")
///     .initial_factor(Temperature(0.3))
///     .service_contract(|f: &Temperature| MyService { temperature: f.0 })
///     .scorer(PassRateScorer)
///     .mutator(StepMutator)
///     .samples_per_iteration(20)
///     .inputs(&inputs)
///     .trial(|uc: &MyService, input| uc.call(input))
///     .objective(Objective::Maximize)
///     .max_iterations(10)
///     .no_improvement_window(3)
///     .build()
///     .run();
/// ```
pub struct OptimizeExperiment<'a, F, T> {
    service_contract_id: String,
    initial_factor: F,
    factory: ServiceContractFactory<'a, F, T>,
    scorer: Box<dyn Scorer>,
    mutator: Box<dyn FactorMutator<F>>,
    objective: Objective,
    samples_per_iteration: u32,
    max_iterations: u32,
    no_improvement_window: u32,
    inputs: &'a [String],
    trial: TrialClosure<'a, T>,
    experiment_id: Option<String>,
}

impl<'a, F, T> OptimizeExperiment<'a, F, T>
where
    F: Clone,
{
    /// Starts a new builder for an optimize experiment.
    ///
    /// Required fields must be set via their corresponding setters
    /// before [`build`](OptimizeExperimentBuilder::build) is called.
    /// Optional fields carry documented defaults.
    #[must_use]
    pub fn builder() -> OptimizeExperimentBuilder<'a, F, T> {
        OptimizeExperimentBuilder::default()
    }

    /// Runs the optimisation and returns the result.
    pub fn run(self) -> OptimizeResult<F> {
        let mut history: Vec<IterationRecord<F>> = Vec::new();
        let mut current_factor = self.initial_factor.clone();
        let mut best_score: Option<f64> = None;
        let mut best_iteration: Option<u32> = None;
        let mut no_improvement_count = 0u32;
        let mut termination_reason = TerminationReason::MaxIterations;

        for iteration in 0..self.max_iterations {
            let service_contract = (self.factory)(&current_factor);

            let config = ExecutionConfig::new(self.samples_per_iteration);
            let recorder = TokenRecorder::new();

            let mut trial_fn = |input: &str| (self.trial)(&service_contract, input);

            let result = ExecutionEngine::run(
                &config,
                self.inputs,
                &recorder,
                crate::controls::run::current(),
                &mut trial_fn,
            );

            let score = self.scorer.score(&result);

            history.push(IterationRecord {
                iteration,
                factor: current_factor.clone(),
                score,
                successes: result.summary().successes(),
                failures: result.summary().failures(),
            });

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

            if no_improvement_count >= self.no_improvement_window {
                termination_reason = TerminationReason::NoImprovement;
                break;
            }

            if iteration + 1 < self.max_iterations {
                current_factor = self.mutator.mutate(&current_factor, &history);
            }
        }

        OptimizeResult {
            service_contract_id: self.service_contract_id,
            objective: self.objective,
            experiment_id: self.experiment_id,
            history,
            best_iteration,
            best_score,
            termination_reason,
        }
    }
}

/// Fluent builder for [`OptimizeExperiment`].
///
/// Required fields — `service_contract_id`, `initial_factor`, `service_contract`
/// (factory), `scorer`, `mutator`, `samples_per_iteration`, `inputs`,
/// and `trial` — must be set before [`build`](Self::build) is called.
/// Missing any of them produces a panic naming the field and the
/// setter to call.
///
/// Optional fields (`objective`, `max_iterations`,
/// `no_improvement_window`, `experiment_id`) carry documented defaults.
/// Setters that validate a single value (e.g., positive iteration
/// counts, non-empty inputs) panic at the setter rather than deferring
/// to `build`.
pub struct OptimizeExperimentBuilder<'a, F, T> {
    service_contract_id: Option<String>,
    initial_factor: Option<F>,
    factory: Option<ServiceContractFactory<'a, F, T>>,
    scorer: Option<Box<dyn Scorer>>,
    mutator: Option<Box<dyn FactorMutator<F>>>,
    objective: Objective,
    samples_per_iteration: Option<u32>,
    max_iterations: u32,
    no_improvement_window: u32,
    inputs: Option<&'a [String]>,
    trial: Option<TrialClosure<'a, T>>,
    experiment_id: Option<String>,
}

impl<F, T> Default for OptimizeExperimentBuilder<'_, F, T> {
    fn default() -> Self {
        Self {
            service_contract_id: None,
            initial_factor: None,
            factory: None,
            scorer: None,
            mutator: None,
            objective: Objective::Maximize,
            samples_per_iteration: None,
            max_iterations: 20,
            no_improvement_window: 5,
            inputs: None,
            trial: None,
            experiment_id: None,
        }
    }
}

impl<'a, F, T> OptimizeExperimentBuilder<'a, F, T> {
    // --- required fields ---

    /// Sets the use case identifier.
    ///
    /// Appears in spec YAML and in the output directory layout.
    #[must_use]
    pub fn service_contract_id(mut self, id: impl Into<String>) -> Self {
        self.service_contract_id = Some(id.into());
        self
    }

    /// Sets the starting factor for the first iteration.
    #[must_use]
    pub fn initial_factor(mut self, factor: F) -> Self {
        self.initial_factor = Some(factor);
        self
    }

    /// Sets the use case factory.
    ///
    /// Given a factor, the factory produces one use case instance. The
    /// framework calls the factory once per iteration, runs
    /// `samples_per_iteration` trials against the resulting instance,
    /// then drops it before the next iteration.
    #[must_use]
    pub fn service_contract(mut self, factory: impl Fn(&F) -> T + 'a) -> Self {
        self.factory = Some(Box::new(factory));
        self
    }

    /// Sets the scoring function.
    #[must_use]
    pub fn scorer(mut self, scorer: impl Scorer + 'static) -> Self {
        self.scorer = Some(Box::new(scorer));
        self
    }

    /// Sets the factor-mutation strategy.
    #[must_use]
    pub fn mutator(mut self, mutator: impl FactorMutator<F> + 'static) -> Self {
        self.mutator = Some(Box::new(mutator));
        self
    }

    /// Sets the number of samples to run per iteration.
    ///
    /// # Panics
    ///
    /// Panics if `samples` is zero.
    #[must_use]
    pub fn samples_per_iteration(mut self, samples: u32) -> Self {
        assert!(samples > 0, "samples_per_iteration must be positive, got 0");
        self.samples_per_iteration = Some(samples);
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
    /// The closure receives a reference to the iteration's use case
    /// instance and an input string, and returns a [`TrialOutcome`]. It
    /// may borrow data that outlives the builder (the `'a` lifetime);
    /// it is not required to be `'static`.
    #[must_use]
    pub fn trial(mut self, trial: impl Fn(&T, &str) -> TrialOutcome + 'a) -> Self {
        self.trial = Some(Box::new(trial));
        self
    }

    // --- optional fields ---

    /// Sets the optimisation objective. Default: [`Objective::Maximize`].
    #[must_use]
    pub const fn objective(mut self, objective: Objective) -> Self {
        self.objective = objective;
        self
    }

    /// Sets the maximum iteration count. Default: 20.
    ///
    /// # Panics
    ///
    /// Panics if `max` is zero.
    #[must_use]
    pub fn max_iterations(mut self, max: u32) -> Self {
        assert!(max > 0, "max_iterations must be positive, got 0");
        self.max_iterations = max;
        self
    }

    /// Sets the no-improvement window for plateau termination. Default: 5.
    ///
    /// # Panics
    ///
    /// Panics if `window` is zero.
    #[must_use]
    pub fn no_improvement_window(mut self, window: u32) -> Self {
        assert!(window > 0, "no_improvement_window must be positive, got 0");
        self.no_improvement_window = window;
        self
    }

    /// Sets the experiment identifier. Default: none.
    #[must_use]
    pub fn experiment_id(mut self, id: impl Into<String>) -> Self {
        self.experiment_id = Some(id.into());
        self
    }

    /// Builds the [`OptimizeExperiment`].
    ///
    /// # Panics
    ///
    /// Panics if any required field is missing, naming the field and
    /// the setter that should have been called.
    #[must_use]
    pub fn build(self) -> OptimizeExperiment<'a, F, T> {
        OptimizeExperiment {
            service_contract_id: self
                .service_contract_id
                .expect("service_contract_id must be set via .service_contract_id(...)"),
            initial_factor: self
                .initial_factor
                .expect("initial_factor must be set via .initial_factor(...)"),
            factory: self
                .factory
                .expect("service_contract factory must be set via .service_contract(...)"),
            scorer: self.scorer.expect("scorer must be set via .scorer(...)"),
            mutator: self.mutator.expect("mutator must be set via .mutator(...)"),
            objective: self.objective,
            samples_per_iteration: self
                .samples_per_iteration
                .expect("samples_per_iteration must be set via .samples_per_iteration(...)"),
            max_iterations: self.max_iterations,
            no_improvement_window: self.no_improvement_window,
            inputs: self.inputs.expect("inputs must be set via .inputs(...)"),
            trial: self.trial.expect("trial must be set via .trial(...)"),
            experiment_id: self.experiment_id,
        }
    }
}

/// Result of an optimize experiment.
#[derive(Debug)]
pub struct OptimizeResult<F> {
    service_contract_id: String,
    objective: Objective,
    experiment_id: Option<String>,
    history: Vec<IterationRecord<F>>,
    best_iteration: Option<u32>,
    best_score: Option<f64>,
    termination_reason: TerminationReason,
}

impl<F> OptimizeResult<F> {
    /// The use case identifier.
    #[must_use]
    pub fn service_contract_id(&self) -> &str {
        &self.service_contract_id
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
    pub fn history(&self) -> &[IterationRecord<F>] {
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

    /// The factor that produced the best score.
    #[must_use]
    pub fn best_factor(&self) -> Option<&F> {
        self.best_iteration.and_then(|idx| {
            self.history
                .iter()
                .find(|r| r.iteration == idx)
                .map(|r| &r.factor)
        })
    }

    /// Why optimisation stopped iterating.
    #[must_use]
    pub const fn termination_reason(&self) -> TerminationReason {
        self.termination_reason
    }
}

impl<F: Serialize> OptimizeResult<F> {
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

    /// Writes the optimization YAML artefact under the given output
    /// root.
    ///
    /// The final path is `{root}/{service_contract_id}/{experiment_id}.yaml`.
    /// The default output root is `target/feotest/optimizations/` — see
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

impl<F: fmt::Display> fmt::Display for OptimizeResult<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let objective_label = match self.objective {
            Objective::Maximize => "maximize",
            Objective::Minimize => "minimize",
        };

        writeln!(
            f,
            "OptimizeResult: {} ({objective_label})",
            self.service_contract_id,
        )?;

        if let Some(id) = &self.experiment_id {
            writeln!(f, "  experiment: {id}")?;
        }

        writeln!(f, "  iterations: {}", self.history.len())?;

        if let (Some(best_iter), Some(best_score)) = (self.best_iteration, self.best_score) {
            writeln!(f, "  best: iteration {best_iter}, score {best_score:.4}")?;
            if let Some(factor) = self.best_factor() {
                writeln!(f, "  best factor: {factor}")?;
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
                    record.factor,
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
    use std::time::Duration;

    #[derive(Clone, Serialize)]
    struct Temp(f64);
    impl fmt::Display for Temp {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    struct MockService {
        temperature: f64,
    }

    fn build_service(t: &Temp) -> MockService {
        MockService { temperature: t.0 }
    }

    struct PassRateScorer;
    impl Scorer for PassRateScorer {
        fn score(&self, result: &ExecutionResult) -> f64 {
            result.summary().observed_pass_rate()
        }
    }

    struct StepMutator;
    impl FactorMutator<Temp> for StepMutator {
        fn mutate(&self, current: &Temp, _history: &[IterationRecord<Temp>]) -> Temp {
            Temp(current.0 + 0.1)
        }
    }

    fn always_succeed(_uc: &MockService, _input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::ZERO)
    }

    #[test]
    fn runs_optimization_iterations() {
        let inputs = vec!["input".to_string()];

        let result = OptimizeExperiment::builder()
            .service_contract_id("test-uc")
            .initial_factor(Temp(0.5))
            .service_contract(build_service)
            .scorer(PassRateScorer)
            .mutator(StepMutator)
            .samples_per_iteration(10)
            .inputs(&inputs)
            .trial(always_succeed)
            .max_iterations(5)
            .no_improvement_window(3)
            .experiment_id("temp-tune")
            .build()
            .run();

        assert!(!result.history().is_empty());
        assert!(result.best_score().is_some());
        assert!(result.best_factor().is_some());
        assert_eq!(result.service_contract_id(), "test-uc");
    }

    #[test]
    fn factor_varies_across_iterations() {
        // Verify the mutator receives the current factor and its output
        // appears in the next iteration.
        let inputs = vec!["input".to_string()];

        let result = OptimizeExperiment::builder()
            .service_contract_id("mutation-test")
            .initial_factor(Temp(1.0))
            .service_contract(build_service)
            .scorer(PassRateScorer)
            .mutator(StepMutator)
            .samples_per_iteration(3)
            .inputs(&inputs)
            .trial(always_succeed)
            .max_iterations(3)
            .no_improvement_window(100)
            .build()
            .run();

        let history = result.history();
        assert_eq!(history.len(), 3);
        assert!((history[0].factor().0 - 1.0).abs() < 1e-10);
        assert!((history[1].factor().0 - 1.1).abs() < 1e-10);
        assert!((history[2].factor().0 - 1.2).abs() < 1e-10);
    }

    #[test]
    fn trial_receives_instance_built_from_current_factor() {
        // Trial checks the use case it receives reflects the iteration's
        // factor. Failure here means the factory / recycling pipeline is
        // broken.
        let inputs = vec!["input".to_string()];

        let result = OptimizeExperiment::builder()
            .service_contract_id("instance-test")
            .initial_factor(Temp(0.5))
            .service_contract(build_service)
            .scorer(PassRateScorer)
            .mutator(StepMutator)
            .samples_per_iteration(3)
            .inputs(&inputs)
            .trial(|uc: &MockService, _input| {
                if uc.temperature < 0.7 {
                    TrialOutcome::success(Duration::ZERO)
                } else {
                    TrialOutcome::failure(
                        crate::model::ContractViolation::new("temp", "too hot"),
                        Duration::ZERO,
                    )
                }
            })
            .max_iterations(4)
            .no_improvement_window(100)
            .build()
            .run();

        let history = result.history();
        assert_eq!(history.len(), 4);
        // Iter 0 (T=0.5): all pass. Iter 1 (T=0.6): all pass. Iter 2 (T=0.7+): all fail.
        assert_eq!(history[0].successes(), 3);
        assert_eq!(history[1].successes(), 3);
        assert_eq!(history[2].failures(), 3);
    }

    #[test]
    fn stops_on_plateau() {
        let inputs = vec!["input".to_string()];

        let result = OptimizeExperiment::builder()
            .service_contract_id("plateau-test")
            .initial_factor(Temp(1.0))
            .service_contract(build_service)
            .scorer(PassRateScorer)
            .mutator(StepMutator)
            .samples_per_iteration(5)
            .inputs(&inputs)
            .trial(always_succeed)
            .max_iterations(20)
            .no_improvement_window(3)
            .build()
            .run();

        // All iterations score 1.0. First + 3 no-improvement → stop at 4.
        assert!(result.history().len() <= 4);
        assert_eq!(
            result.termination_reason(),
            TerminationReason::NoImprovement
        );
    }

    #[test]
    fn minimize_objective_is_honoured() {
        let inputs = vec!["input".to_string()];

        let result = OptimizeExperiment::builder()
            .service_contract_id("minimize-test")
            .initial_factor(Temp(1.0))
            .service_contract(build_service)
            .scorer(PassRateScorer)
            .mutator(StepMutator)
            .samples_per_iteration(5)
            .inputs(&inputs)
            .trial(always_succeed)
            .objective(Objective::Minimize)
            .max_iterations(3)
            .build()
            .run();

        assert_eq!(result.objective(), Objective::Minimize);
        assert!(result.best_score().is_some());
    }

    // --- Builder precondition tests (setter-level validation) ---

    #[test]
    #[should_panic(expected = "samples_per_iteration must be positive")]
    fn rejects_zero_samples_per_iteration() {
        let _ = OptimizeExperiment::<Temp, MockService>::builder().samples_per_iteration(0);
    }

    #[test]
    #[should_panic(expected = "max_iterations must be positive")]
    fn rejects_zero_max_iterations() {
        let _ = OptimizeExperiment::<Temp, MockService>::builder().max_iterations(0);
    }

    #[test]
    #[should_panic(expected = "no_improvement_window must be positive")]
    fn rejects_zero_no_improvement_window() {
        let _ = OptimizeExperiment::<Temp, MockService>::builder().no_improvement_window(0);
    }

    #[test]
    #[should_panic(expected = "inputs must not be empty")]
    fn rejects_empty_inputs() {
        let empty: Vec<String> = vec![];
        let _ = OptimizeExperiment::<Temp, MockService>::builder().inputs(&empty);
    }

    // --- Builder precondition tests (missing-required at build) ---

    #[test]
    #[should_panic(expected = "service_contract_id must be set")]
    fn build_without_any_required_fields_panics() {
        let _ = OptimizeExperiment::<Temp, MockService>::builder().build();
    }
}
