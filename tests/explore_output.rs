//! Integration tests for exploration YAML output (EX05).

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use feotest::experiment::ExploreExperiment;
use feotest::model::TrialOutcome;
use feotest::spec::explore::{ExplorationSpec, ExploreSpecWriter, FactorYamlValue};
use feotest::usecase::UseCase;

struct MockService {
    id: &'static str,
    label: String,
}

impl MockService {
    fn new(id: &'static str, label: &str) -> Self {
        Self {
            id,
            label: label.to_owned(),
        }
    }
}

impl UseCase for MockService {
    fn id(&self) -> &str {
        self.id
    }
}

impl fmt::Display for MockService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[test]
fn explore_writes_per_config_yaml_files() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["request".to_string()];

    let svc_a = MockService::new("test-uc", "config-a");
    let svc_b = MockService::new("test-uc", "config-b");

    let result = ExploreExperiment::new(&svc_a, 5, &inputs, |_svc: &MockService, _input| {
        TrialOutcome::success(Duration::from_millis(1))
    })
    .config(&svc_b)
    .experiment_id("test-explore")
    .output_dir(dir.path())
    .run();

    let paths = result.spec_paths().expect("spec paths should be set");
    assert_eq!(paths.len(), 2);

    for path in paths {
        assert!(path.exists(), "spec file should exist: {}", path.display());
        assert!(
            path.extension().is_some_and(|ext| ext == "yaml"),
            "should have .yaml extension"
        );
    }

    let explore_dir = dir.path().join("test-uc");
    assert!(explore_dir.exists());
    assert!(explore_dir.join("config-a.yaml").exists());
    assert!(explore_dir.join("config-b.yaml").exists());
}

#[test]
fn explore_yaml_contains_correct_content() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["request".to_string()];

    let svc = MockService::new("content-test", "all-pass");

    let result =
        ExploreExperiment::new(&svc, 10, &inputs, |_svc: &MockService, _input| {
            TrialOutcome::success(Duration::from_millis(5))
        })
        .output_dir(dir.path())
        .run();

    let paths = result.spec_paths().unwrap();
    let yaml_content = std::fs::read_to_string(&paths[0]).unwrap();
    let spec: ExplorationSpec = ExplorationSpec::from_yaml(&yaml_content).unwrap();

    assert_eq!(spec.schema_version, "feotest-spec-1");
    assert_eq!(spec.use_case_id, "content-test");
    assert_eq!(spec.statistics.successes, 10);
    assert_eq!(spec.statistics.failures, 0);
    assert!((spec.statistics.observed - 1.0).abs() < 1e-10);
}

#[test]
fn explore_yaml_includes_factor_values() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["request".to_string()];

    let svc = MockService::new("factor-test", "gpt-4_temp-0.7");

    let factors = BTreeMap::from([
        (
            "model".to_owned(),
            FactorYamlValue::String("gpt-4".to_owned()),
        ),
        ("temperature".to_owned(), FactorYamlValue::Float(0.7)),
    ]);

    let result = ExploreExperiment::new(&svc, 5, &inputs, |_svc: &MockService, _input| {
        TrialOutcome::success(Duration::from_millis(1))
    })
    .factors("gpt-4_temp-0.7", factors)
    .output_dir(dir.path())
    .run();

    let paths = result.spec_paths().unwrap();
    let yaml_content = std::fs::read_to_string(&paths[0]).unwrap();
    let spec: ExplorationSpec = ExplorationSpec::from_yaml(&yaml_content).unwrap();

    assert_eq!(spec.execution_context.len(), 2);
    assert!(yaml_content.contains("executionContext"));
    assert!(yaml_content.contains("model"));
    assert!(yaml_content.contains("temperature"));
}

#[test]
fn explore_without_output_dir_produces_no_files() {
    let inputs = vec!["request".to_string()];
    let svc = MockService::new("no-output-test", "no-output");

    let result = ExploreExperiment::new(&svc, 5, &inputs, |_svc: &MockService, _input| {
        TrialOutcome::success(Duration::from_millis(1))
    })
    .run();

    assert!(result.spec_paths().is_none());
}

#[test]
fn explore_spec_writer_standalone() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let svc = MockService::new("writer-test", "standalone");

    let result = ExploreExperiment::new(&svc, 5, &inputs, |_svc: &MockService, _input| {
        TrialOutcome::success(Duration::from_millis(1))
    })
    .run();

    let writer = ExploreSpecWriter::new(dir.path());
    let factors = BTreeMap::new();
    let paths = writer.write_all(&result, &factors).unwrap();

    assert_eq!(paths.len(), 1);
    assert!(paths[0].exists());
}

#[test]
fn explore_yaml_is_descriptive_not_inferential() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let svc = MockService::new("desc-test", "descriptive");

    let result = ExploreExperiment::new(&svc, 5, &inputs, |_svc: &MockService, _input| {
        TrialOutcome::success(Duration::from_millis(1))
    })
    .output_dir(dir.path())
    .run();

    let paths = result.spec_paths().unwrap();
    let yaml = std::fs::read_to_string(&paths[0]).unwrap();

    assert!(!yaml.contains("standardError"));
    assert!(!yaml.contains("confidenceInterval"));
    assert!(!yaml.contains("minPassRate"));
    assert!(!yaml.contains("requirements"));

    assert!(yaml.contains("observed"));
    assert!(yaml.contains("successes"));
    assert!(yaml.contains("failures"));
}
