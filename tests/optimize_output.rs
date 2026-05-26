//! Integration tests for optimization YAML output.

use feotest::controls::Cost;
use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::{
    ContractExecutionResult, FactorMutator, IterationRecord, Objective, OptimizeExperiment, Scorer,
    TerminationReason,
};
use feotest::model::Defect;
use feotest::service_contract::ServiceContract;
use feotest::spec::optimization::OptimizationSpec;
use serde::Serialize;

// ---------------------------------------------------------------------------
// Factor and service contract types shared by most tests
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
struct Temp(f64);

#[derive(Clone, Serialize)]
struct Prompt(String);

/// An always-pass contract; iterations differ only by the factor that built it.
struct Service;

impl ServiceContract for Service {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        "service"
    }

    fn invoke(&self, input: &String, _cost: &mut Cost) -> Result<String, Defect> {
        Ok(input.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([Criterion::meeting()
            .pass_rate(0.5)
            .name("ok")
            .satisfies("ok", |_: &String| Ok(()))
            .build()])
    }
}

fn build_service_from_temp(_t: &Temp) -> Service {
    Service
}

fn build_service_from_prompt(_p: &Prompt) -> Service {
    Service
}

struct PassRateScorer;
impl Scorer for PassRateScorer {
    fn score(&self, result: &ContractExecutionResult) -> f64 {
        result.summary().observed_pass_rate()
    }
}

struct FloatIncrementMutator(f64);
impl FactorMutator<Temp> for FloatIncrementMutator {
    fn mutate(&self, current: &Temp, _history: &[IterationRecord<Temp>]) -> Temp {
        Temp(current.0 + self.0)
    }
}

struct PromptMutator {
    variants: Vec<String>,
}
impl FactorMutator<Prompt> for PromptMutator {
    fn mutate(&self, _current: &Prompt, history: &[IterationRecord<Prompt>]) -> Prompt {
        let idx = history.len().min(self.variants.len() - 1);
        Prompt(self.variants[idx].clone())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn writes_yaml_to_service_contract_scoped_path() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let result = OptimizeExperiment::builder()
        .service_contract_id("shopping-basket")
        .initial_factor(Temp(0.1))
        .service_contract(build_service_from_temp)
        .scorer(PassRateScorer)
        .mutator(FloatIncrementMutator(0.1))
        .samples_per_iteration(5)
        .inputs(&inputs)
        .max_iterations(3)
        .no_improvement_window(10)
        .experiment_id("temp-tune-v1")
        .build()
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
    assert_eq!(spec.service_contract_id, "shopping-basket");
    assert_eq!(spec.experiment_id, "temp-tune-v1");
    assert_eq!(spec.objective, "MAXIMIZE");
    assert_eq!(spec.iterations.len(), 3);
    assert_eq!(spec.convergence.total_iterations, 3);
    assert_eq!(spec.convergence.best_iteration, Some(0));
}

#[test]
fn multi_line_factor_value_uses_block_scalar() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let prompts = vec![
        "You are a helpful assistant.\nAlways be polite.".to_string(),
        "You are a shopping assistant.\nAlways suggest related items.\nReturn JSON.".to_string(),
    ];

    let result = OptimizeExperiment::builder()
        .service_contract_id("prompt-tune")
        .initial_factor(Prompt(prompts[0].clone()))
        .service_contract(build_service_from_prompt)
        .scorer(PassRateScorer)
        .mutator(PromptMutator { variants: prompts })
        .samples_per_iteration(3)
        .inputs(&inputs)
        .max_iterations(2)
        .no_improvement_window(10)
        .experiment_id("prompt-v1")
        .build()
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

    let spec = OptimizationSpec::from_yaml(&yaml).unwrap();
    assert_eq!(spec.iterations.len(), 2);
}

#[test]
fn minimize_objective_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let result = OptimizeExperiment::builder()
        .service_contract_id("cost-min")
        .initial_factor(Temp(100.0))
        .service_contract(build_service_from_temp)
        .scorer(PassRateScorer)
        .mutator(FloatIncrementMutator(-10.0))
        .samples_per_iteration(3)
        .inputs(&inputs)
        .objective(Objective::Minimize)
        .max_iterations(2)
        .no_improvement_window(10)
        .experiment_id("cost-tune")
        .build()
        .run();

    let path = result.write_to(dir.path()).unwrap();
    let spec = OptimizationSpec::from_yaml(&std::fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(spec.objective, "MINIMIZE");
}

#[test]
fn plateau_termination_recorded_as_no_improvement() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    // All iterations score 1.0, so after the first + no_improvement_window=2
    // the run terminates on plateau.
    let result = OptimizeExperiment::builder()
        .service_contract_id("plateau-case")
        .initial_factor(Temp(1.0))
        .service_contract(build_service_from_temp)
        .scorer(PassRateScorer)
        .mutator(FloatIncrementMutator(0.1))
        .samples_per_iteration(3)
        .inputs(&inputs)
        .max_iterations(20)
        .no_improvement_window(2)
        .experiment_id("plateau-exp")
        .build()
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

    // no_improvement_window exceeds max_iterations, so the run always exits
    // via the iteration cap rather than on plateau.
    let result = OptimizeExperiment::builder()
        .service_contract_id("max-iter")
        .initial_factor(Temp(1.0))
        .service_contract(build_service_from_temp)
        .scorer(PassRateScorer)
        .mutator(FloatIncrementMutator(0.1))
        .samples_per_iteration(3)
        .inputs(&inputs)
        .max_iterations(3)
        .no_improvement_window(100)
        .experiment_id("cap-exp")
        .build()
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

    let result = OptimizeExperiment::builder()
        .service_contract_id("samples-test")
        .initial_factor(Temp(1.0))
        .service_contract(build_service_from_temp)
        .scorer(PassRateScorer)
        .mutator(FloatIncrementMutator(0.1))
        .samples_per_iteration(7)
        .inputs(&inputs)
        .max_iterations(1)
        .experiment_id("samples-exp")
        .build()
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

    let result = OptimizeExperiment::builder()
        .service_contract_id("yaml-only")
        .initial_factor(Temp(0.5))
        .service_contract(build_service_from_temp)
        .scorer(PassRateScorer)
        .mutator(FloatIncrementMutator(0.1))
        .samples_per_iteration(3)
        .inputs(&inputs)
        .max_iterations(2)
        .experiment_id("in-memory")
        .build()
        .run();

    let yaml = result.to_yaml().unwrap();
    assert!(yaml.contains("schemaVersion: feotest-spec-1"));
    assert!(yaml.contains("useCaseId: yaml-only"));
}

#[test]
fn struct_factor_emits_as_yaml_mapping() {
    // Demonstrates that multi-field struct factors serialise cleanly.
    #[derive(Clone, Serialize)]
    struct ModelAndTemp {
        model: &'static str,
        temperature: f64,
    }

    struct ModelTempMutator;
    impl FactorMutator<ModelAndTemp> for ModelTempMutator {
        fn mutate(
            &self,
            current: &ModelAndTemp,
            _history: &[IterationRecord<ModelAndTemp>],
        ) -> ModelAndTemp {
            ModelAndTemp {
                model: current.model,
                temperature: current.temperature + 0.1,
            }
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let result = OptimizeExperiment::builder()
        .service_contract_id("struct-factor")
        .initial_factor(ModelAndTemp {
            model: "gpt-4",
            temperature: 0.3,
        })
        .service_contract(|_f: &ModelAndTemp| Service)
        .scorer(PassRateScorer)
        .mutator(ModelTempMutator)
        .samples_per_iteration(3)
        .inputs(&inputs)
        .max_iterations(2)
        .experiment_id("struct-v1")
        .build()
        .run();

    let path = result.write_to(dir.path()).unwrap();
    let yaml = std::fs::read_to_string(&path).unwrap();

    assert!(
        yaml.contains("model: gpt-4"),
        "struct factor should serialise as a YAML mapping; got:\n{yaml}"
    );
    assert!(yaml.contains("temperature:"));
}
