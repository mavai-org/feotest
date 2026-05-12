//! Integration tests for exploration YAML output (EX05).

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use feotest::experiment::ExploreExperiment;
use feotest::model::TrialOutcome;
use feotest::spec::explore::{ExplorationSpec, ExploreSpecWriter, FactorYamlValue};

/// A factor variant for exploration. `Display` supplies the
/// configuration name used in filenames and YAML.
#[derive(Clone)]
struct ConfigFactor {
    label: &'static str,
}
impl fmt::Display for ConfigFactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

/// A minimal service contract that the factory produces from each factor.
struct MockService;

fn mock_service_factory(_factor: &ConfigFactor) -> MockService {
    MockService
}

#[test]
fn explore_writes_per_config_yaml_files() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["request".to_string()];

    let factors = vec![
        ConfigFactor { label: "config-a" },
        ConfigFactor { label: "config-b" },
    ];

    let result = ExploreExperiment::builder()
        .service_contract_id("test-uc")
        .factors(factors)
        .service_contract(mock_service_factory)
        .samples_per_config(5)
        .inputs(&inputs)
        .trial(|_svc: &MockService, _input| TrialOutcome::success(Duration::from_millis(1)))
        .experiment_id("test-explore")
        .output_dir(dir.path())
        .build()
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

    let factors = vec![ConfigFactor { label: "all-pass" }];

    let result = ExploreExperiment::builder()
        .service_contract_id("content-test")
        .factors(factors)
        .service_contract(mock_service_factory)
        .samples_per_config(10)
        .inputs(&inputs)
        .trial(|_svc: &MockService, _input| TrialOutcome::success(Duration::from_millis(5)))
        .output_dir(dir.path())
        .build()
        .run();

    let paths = result.spec_paths().unwrap();
    let yaml_content = std::fs::read_to_string(&paths[0]).unwrap();
    let spec: ExplorationSpec = ExplorationSpec::from_yaml(&yaml_content).unwrap();

    assert_eq!(spec.schema_version, "feotest-spec-1");
    assert_eq!(spec.service_contract_id, "content-test");
    assert_eq!(spec.statistics.successes, 10);
    assert_eq!(spec.statistics.failures, 0);
    assert!((spec.statistics.observed - 1.0).abs() < 1e-10);
}

#[test]
fn explore_without_output_dir_produces_no_files() {
    let inputs = vec!["request".to_string()];
    let factors = vec![ConfigFactor { label: "no-output" }];

    let result = ExploreExperiment::builder()
        .service_contract_id("no-output-test")
        .factors(factors)
        .service_contract(mock_service_factory)
        .samples_per_config(5)
        .inputs(&inputs)
        .trial(|_svc: &MockService, _input| TrialOutcome::success(Duration::from_millis(1)))
        .build()
        .run();

    assert!(result.spec_paths().is_none());
}

#[test]
fn explore_spec_writer_standalone() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let factors = vec![ConfigFactor {
        label: "standalone",
    }];

    let result = ExploreExperiment::builder()
        .service_contract_id("writer-test")
        .factors(factors)
        .service_contract(mock_service_factory)
        .samples_per_config(5)
        .inputs(&inputs)
        .trial(|_svc: &MockService, _input| TrialOutcome::success(Duration::from_millis(1)))
        .build()
        .run();

    let writer = ExploreSpecWriter::new(dir.path());
    let empty_factor_values: BTreeMap<String, BTreeMap<String, FactorYamlValue>> = BTreeMap::new();
    let paths = writer.write_all(&result, &empty_factor_values).unwrap();

    assert_eq!(paths.len(), 1);
    assert!(paths[0].exists());
}

#[test]
fn explore_yaml_is_descriptive_not_inferential() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let factors = vec![ConfigFactor {
        label: "descriptive",
    }];

    let result = ExploreExperiment::builder()
        .service_contract_id("desc-test")
        .factors(factors)
        .service_contract(mock_service_factory)
        .samples_per_config(5)
        .inputs(&inputs)
        .trial(|_svc: &MockService, _input| TrialOutcome::success(Duration::from_millis(1)))
        .output_dir(dir.path())
        .build()
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
