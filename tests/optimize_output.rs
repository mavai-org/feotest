//! Integration tests for optimization YAML output (EX06).

use std::time::Duration;

use feotest::experiment::{
    ExecutionResult, FactorMutator, Objective, OptimizeExperiment, Scorer, TerminationReason,
};
use feotest::model::TrialOutcome;
use feotest::spec::optimization::OptimizationSpec;
use feotest::usecase::{FactorValue, UseCase};

struct PassRateScorer;
impl Scorer for PassRateScorer {
    fn score(&self, result: &ExecutionResult) -> f64 {
        result.summary().observed_pass_rate()
    }
}

struct FloatIncrementMutator(f64);
impl FactorMutator for FloatIncrementMutator {
    fn mutate(
        &self,
        current: &FactorValue,
        _history: &[feotest::experiment::IterationRecord],
    ) -> FactorValue {
        match current {
            FactorValue::Float(v) => FactorValue::Float(v + self.0),
            other => other.clone(),
        }
    }
}

struct PromptMutator {
    variants: Vec<String>,
}
impl FactorMutator for PromptMutator {
    fn mutate(
        &self,
        _current: &FactorValue,
        history: &[feotest::experiment::IterationRecord],
    ) -> FactorValue {
        let idx = history.len().min(self.variants.len() - 1);
        FactorValue::String(self.variants[idx].clone())
    }
}

struct SimpleUseCase(&'static str);
impl UseCase for SimpleUseCase {
    fn id(&self) -> &str {
        self.0
    }
}

#[test]
fn writes_yaml_to_use_case_scoped_path() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let uc = SimpleUseCase("shopping-basket");

    let result = OptimizeExperiment::new(
        &uc,
        "temperature",
        FactorValue::Float(0.1),
        PassRateScorer,
        FloatIncrementMutator(0.1),
        &inputs,
        |_input| TrialOutcome::success(Duration::ZERO),
        |_value| {},
    )
    .with_max_iterations(3)
    .with_samples_per_iteration(5)
    .with_no_improvement_window(10)
    .with_experiment_id("temp-tune-v1")
    .run();

    let path = result.write_to(dir.path()).unwrap();

    assert!(path.exists());
    assert_eq!(
        path.strip_prefix(dir.path()).unwrap(),
        std::path::Path::new("shopping-basket/temp-tune-v1.yaml")
    );

    let yaml = std::fs::read_to_string(&path).unwrap();
    let spec = OptimizationSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec.schema_version, "feotest-spec-1");
    assert_eq!(spec.use_case_id, "shopping-basket");
    assert_eq!(spec.experiment_id, "temp-tune-v1");
    assert_eq!(spec.control_factor, "temperature");
    assert_eq!(spec.objective, "MAXIMIZE");
    assert_eq!(spec.iterations.len(), 3);
    assert_eq!(spec.convergence.total_iterations, 3);
    assert_eq!(spec.convergence.best_iteration, Some(0));
}

#[test]
fn multi_line_factor_value_uses_block_scalar() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let uc = SimpleUseCase("prompt-tune");

    let prompts = vec![
        "You are a helpful assistant.\nAlways be polite.".to_string(),
        "You are a shopping assistant.\nAlways suggest related items.\nReturn JSON.".to_string(),
    ];

    let result = OptimizeExperiment::new(
        &uc,
        "systemPrompt",
        FactorValue::String(prompts[0].clone()),
        PassRateScorer,
        PromptMutator { variants: prompts },
        &inputs,
        |_input| TrialOutcome::success(Duration::ZERO),
        |_value| {},
    )
    .with_max_iterations(2)
    .with_samples_per_iteration(3)
    .with_no_improvement_window(10)
    .with_experiment_id("prompt-v1")
    .run();

    let path = result.write_to(dir.path()).unwrap();
    let yaml = std::fs::read_to_string(&path).unwrap();

    // Literal block scalar marker, not escaped newlines.
    assert!(
        yaml.contains("factorValue: |") || yaml.contains("factorValue: |-"),
        "expected block scalar; got:\n{yaml}"
    );
    assert!(
        !yaml.contains("\\n"),
        "factor value should not be escaped:\n{yaml}"
    );

    // Deserialisation preserves the full multi-line content.
    let spec = OptimizationSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec.iterations.len(), 2);
}

#[test]
fn minimize_objective_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let uc = SimpleUseCase("cost-min");

    let result = OptimizeExperiment::new(
        &uc,
        "latencyMs",
        FactorValue::Float(100.0),
        PassRateScorer,
        FloatIncrementMutator(-10.0),
        &inputs,
        |_input| TrialOutcome::success(Duration::ZERO),
        |_value| {},
    )
    .with_objective(Objective::Minimize)
    .with_max_iterations(2)
    .with_samples_per_iteration(3)
    .with_no_improvement_window(10)
    .with_experiment_id("cost-tune")
    .run();

    let path = result.write_to(dir.path()).unwrap();
    let spec = OptimizationSpec::from_yaml(&std::fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(spec.objective, "MINIMIZE");
}

#[test]
fn plateau_termination_recorded_as_no_improvement() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let uc = SimpleUseCase("plateau-case");

    // All iterations score 1.0, so after the first + no_improvement_window=2
    // the run terminates on plateau.
    let result = OptimizeExperiment::new(
        &uc,
        "factor",
        FactorValue::Float(1.0),
        PassRateScorer,
        FloatIncrementMutator(0.1),
        &inputs,
        |_input| TrialOutcome::success(Duration::ZERO),
        |_value| {},
    )
    .with_max_iterations(20)
    .with_samples_per_iteration(3)
    .with_no_improvement_window(2)
    .with_experiment_id("plateau-exp")
    .run();

    assert_eq!(
        result.termination_reason(),
        TerminationReason::NoImprovement
    );

    let path = result.write_to(dir.path()).unwrap();
    let spec = OptimizationSpec::from_yaml(&std::fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(spec.convergence.termination_reason, "NO_IMPROVEMENT");
    assert!(spec.iterations.len() < 20);
}

#[test]
fn max_iterations_termination_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let uc = SimpleUseCase("max-iter");

    // no_improvement_window exceeds max_iterations, so the run always exits
    // via the iteration cap rather than on plateau.
    let result = OptimizeExperiment::new(
        &uc,
        "factor",
        FactorValue::Float(1.0),
        PassRateScorer,
        FloatIncrementMutator(0.1),
        &inputs,
        |_input| TrialOutcome::success(Duration::ZERO),
        |_value| {},
    )
    .with_max_iterations(3)
    .with_samples_per_iteration(3)
    .with_no_improvement_window(100)
    .with_experiment_id("cap-exp")
    .run();

    assert_eq!(
        result.termination_reason(),
        TerminationReason::MaxIterations
    );

    let path = result.write_to(dir.path()).unwrap();
    let spec = OptimizationSpec::from_yaml(&std::fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(spec.convergence.termination_reason, "MAX_ITERATIONS");
    assert_eq!(spec.convergence.total_iterations, 3);
}

#[test]
fn iterations_record_samples_executed() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let uc = SimpleUseCase("samples-test");

    let result = OptimizeExperiment::new(
        &uc,
        "factor",
        FactorValue::Float(1.0),
        PassRateScorer,
        FloatIncrementMutator(0.1),
        &inputs,
        |_input| TrialOutcome::success(Duration::ZERO),
        |_value| {},
    )
    .with_max_iterations(1)
    .with_samples_per_iteration(7)
    .with_experiment_id("samples-exp")
    .run();

    let path = result.write_to(dir.path()).unwrap();
    let spec = OptimizationSpec::from_yaml(&std::fs::read_to_string(&path).unwrap()).unwrap();

    let iter0 = &spec.iterations[0];
    assert_eq!(iter0.successes + iter0.failures, 7);
    assert_eq!(iter0.samples_executed, 7);
}

#[test]
fn result_to_yaml_without_writing_to_disk() {
    let inputs = vec!["input".to_string()];
    let uc = SimpleUseCase("yaml-only");

    let result = OptimizeExperiment::new(
        &uc,
        "factor",
        FactorValue::Float(0.5),
        PassRateScorer,
        FloatIncrementMutator(0.1),
        &inputs,
        |_input| TrialOutcome::success(Duration::ZERO),
        |_value| {},
    )
    .with_max_iterations(2)
    .with_samples_per_iteration(3)
    .with_experiment_id("in-memory")
    .run();

    let yaml = result.to_yaml().unwrap();
    assert!(yaml.contains("schemaVersion: feotest-spec-1"));
    assert!(yaml.contains("useCaseId: yaml-only"));
}
