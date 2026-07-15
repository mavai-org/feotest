//! Emitter conformance for the exploration interchange artefact.
//!
//! The exploration YAML this crate emits is a family-level interchange
//! format, consumed by tooling that is not this crate. These tests validate
//! real emitted artefacts against the vendored copy of the published JSON
//! Schema (`tests/conformance/interchange/mavai-explore-1.schema.json`,
//! pinned per family schema release), plus the semantic obligations the
//! schema cannot express: the sortedness of the latency vector and the
//! minimum-sample gating of the stated percentiles.

use std::fmt;
use std::path::Path;

use feotest::controls::Cost;
use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::ExploreExperiment;
use feotest::model::{ContractViolation, Defect};
use feotest::service_contract::ServiceContract;
use feotest::spec::explore::ExplorationSpec;
use feotest::statistics::latency::min_samples_for;

/// A factor variant; `Display` names the configuration.
#[derive(Clone)]
struct ConfigFactor {
    label: &'static str,
}
impl fmt::Display for ConfigFactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

/// A service that fails the `well-formed` check whenever the input says so —
/// exercising both the failure distribution and the passing-latency subset.
struct FlakyService;

impl ServiceContract for FlakyService {
    type Input = String;
    type Output = String;

    fn id(&self) -> &'static str {
        "flaky"
    }

    fn invoke(&self, input: &String, _cost: &mut Cost) -> Result<String, Defect> {
        Ok(input.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([Criterion::meeting()
            .pass_rate(0.5)
            .name("well-formed")
            .satisfies("well-formed", |response: &String| {
                if response.contains("bad") {
                    Err(ContractViolation::new("well-formed", "malformed response"))
                } else {
                    Ok(())
                }
            })
            .build()])
    }
}

/// Runs a small exploration and returns the emitted YAML documents.
fn emit_artefacts(dir: &Path) -> Vec<String> {
    let inputs = vec![
        "ok-1".to_string(),
        "bad-2".to_string(),
        "ok-3".to_string(),
        "ok-4".to_string(),
        "ok-5".to_string(),
        "ok-6".to_string(),
    ];
    let factors = vec![
        ConfigFactor { label: "config-a" },
        ConfigFactor { label: "config-b" },
    ];
    let result = ExploreExperiment::builder()
        .service_contract_id("flaky-service")
        .factors(factors)
        .service_contract(|_: &ConfigFactor| FlakyService)
        .samples_per_config(6)
        .inputs(&inputs)
        .experiment_id("conformance-run")
        .output_dir(dir)
        .build()
        .run();

    result
        .spec_paths()
        .expect("output_dir was set, so paths must be recorded")
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect()
}

/// The vendored published schema, compiled.
fn published_schema() -> jsonschema::Validator {
    let schema_text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/conformance/interchange/mavai-explore-1.schema.json"
    ))
    .unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_text).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

/// A YAML document as a JSON value, for schema validation.
fn as_json(yaml: &str) -> serde_json::Value {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
    serde_json::to_value(value).unwrap()
}

#[test]
fn emitted_artefacts_validate_against_the_published_schema() {
    let dir = tempfile::tempdir().unwrap();
    let artefacts = emit_artefacts(dir.path());
    assert_eq!(artefacts.len(), 2);

    let validator = published_schema();
    for yaml in &artefacts {
        let document = as_json(yaml);
        let errors: Vec<String> = validator
            .iter_errors(&document)
            .map(|error| format!("{} at {}", error, error.instance_path()))
            .collect();
        assert!(
            errors.is_empty(),
            "emitted artefact violates the published schema:\n{}",
            errors.join("\n")
        );
    }
}

#[test]
fn emitted_latency_detail_honours_the_semantic_obligations() {
    let dir = tempfile::tempdir().unwrap();
    for yaml in emit_artefacts(dir.path()) {
        let spec = ExplorationSpec::from_yaml(&yaml).unwrap();
        let latency = spec.latency.expect("passing samples were recorded");

        // The vector is sorted ascending and matches the contributing count.
        assert!(
            latency
                .sorted_passing_latencies_ms
                .windows(2)
                .all(|pair| pair[0] <= pair[1]),
            "latency vector must be sorted ascending"
        );
        assert_eq!(
            latency.sorted_passing_latencies_ms.len(),
            latency.contributing_samples as usize
        );
        assert!(latency.contributing_samples <= latency.total_samples);

        // Each percentile is stated iff the passing count clears its floor.
        let passing = latency.contributing_samples;
        for (stated, fraction) in [
            (latency.p50_ms, 0.50),
            (latency.p95_ms, 0.95),
            (latency.p99_ms, 0.99),
        ] {
            assert_eq!(
                stated.is_some(),
                passing >= min_samples_for(fraction),
                "p{fraction} statedness must follow the minimum-sample gate at n={passing}",
            );
        }
    }
}

#[test]
fn emitted_criteria_carry_names_and_consistent_tallies() {
    let dir = tempfile::tempdir().unwrap();
    for yaml in emit_artefacts(dir.path()) {
        let spec = ExplorationSpec::from_yaml(&yaml).unwrap();
        assert_eq!(spec.schema_version, "mavai-explore-1");
        assert!(
            spec.configuration
                .as_deref()
                .is_some_and(|name| name.starts_with("config-")),
            "configuration display name must be carried in the body"
        );

        let criteria = spec.statistics.criteria.expect("criteria are binding");
        let well_formed = criteria.get("well-formed").expect("declared criterion");
        assert_eq!(
            well_formed.pass + well_formed.fail,
            spec.execution.samples_executed
        );
        // One in six inputs fails, so the distribution names the check.
        assert_eq!(spec.statistics.failures, 1);
        let distribution = spec
            .statistics
            .failure_distribution
            .expect("failures > 0 requires the distribution");
        assert_eq!(distribution.get("well-formed"), Some(&1));
    }
}
