//! Emitter conformance for the exploration and optimization interchange
//! artefacts.
//!
//! The experiment YAML this crate emits is a family-level interchange
//! format, consumed by tooling that is not this crate. These tests validate
//! real emitted artefacts against the vendored copies of the published JSON
//! Schemas (`tests/conformance/interchange/mavai-explore-1.schema.json` and
//! `mavai-optimize-1.schema.json`, pinned per family schema release), plus
//! the semantic obligations the schemas cannot express: the sortedness of
//! the latency vector, the minimum-sample gating of the stated percentiles,
//! and the optimize convergence block's internal consistency with the
//! iteration it names.

use std::fmt;
use std::path::Path;

use feotest::controls::Cost;
use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::{
    ExploreExperiment, FactorMutator, IterationRecord, ObservedPassRate, OptimizeExperiment,
};
use feotest::model::{ContractViolation, Defect};
use feotest::service_contract::ServiceContract;
use feotest::spec::explore::ExplorationSpec;
use feotest::spec::optimization::OptimizationSpec;
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

/// A vendored published schema, compiled.
fn published_schema(name: &str) -> jsonschema::Validator {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/conformance/interchange")
        .join(name);
    let schema_text = std::fs::read_to_string(path).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_text).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

/// Asserts one YAML document validates against a vendored schema.
fn assert_validates(schema_name: &str, yaml: &str) {
    let validator = published_schema(schema_name);
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

    for yaml in &artefacts {
        assert_validates("mavai-explore-1.schema.json", yaml);
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

// ── optimization ────────────────────────────────────────────────────────────

/// A scalar factor for optimize runs; more bad inputs fail at higher indices.
#[derive(Clone, serde::Serialize)]
struct Strictness(u32);

/// Steps the strictness up by one each iteration.
struct StepMutator;
impl FactorMutator<Strictness> for StepMutator {
    fn mutate(&self, current: &Strictness, _history: &[IterationRecord<Strictness>]) -> Strictness {
        Strictness(current.0 + 1)
    }
}

/// Runs a small real optimize experiment and returns the emitted YAML.
fn emit_optimize_artefact(dir: &Path) -> String {
    let inputs = vec![
        "ok-1".to_string(),
        "bad-2".to_string(),
        "ok-3".to_string(),
        "ok-4".to_string(),
        "ok-5".to_string(),
        "ok-6".to_string(),
    ];
    let result = OptimizeExperiment::builder()
        .service_contract_id("flaky-service")
        .initial_factor(Strictness(0))
        .service_contract(|_: &Strictness| FlakyService)
        .scorer(ObservedPassRate)
        .mutator(StepMutator)
        .samples_per_iteration(6)
        .inputs(&inputs)
        .max_iterations(3)
        .no_improvement_window(10)
        .experiment_id("conformance-optimize")
        .build()
        .run();

    let path = result.write_to(dir).unwrap();
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn emitted_optimize_artefact_validates_against_the_published_schema() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = emit_optimize_artefact(dir.path());
    assert_validates("mavai-optimize-1.schema.json", &yaml);
}

#[test]
fn optimize_iterations_honour_the_latency_and_statistics_obligations() {
    let dir = tempfile::tempdir().unwrap();
    let spec = OptimizationSpec::from_yaml(&emit_optimize_artefact(dir.path())).unwrap();

    for iteration in &spec.iterations {
        // One in six inputs fails, so every iteration has passing samples,
        // a positive failure count, and the distribution naming the check.
        let latency = iteration
            .latency
            .as_ref()
            .expect("passing samples were recorded");
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

        let criteria = iteration
            .statistics
            .criteria
            .as_ref()
            .expect("criteria are binding");
        let well_formed = criteria.get("well-formed").expect("declared criterion");
        assert_eq!(
            well_formed.pass + well_formed.fail,
            iteration.execution.samples_executed
        );
        assert_eq!(iteration.statistics.failures, 1);
        let distribution = iteration
            .statistics
            .failure_distribution
            .as_ref()
            .expect("failures > 0 requires the distribution");
        assert_eq!(distribution.len(), 1);
        assert_eq!(distribution[0].condition, "well-formed");
        assert_eq!(distribution[0].count, 1);
    }
}

#[test]
fn optimize_convergence_is_internally_consistent_and_the_scorer_is_stated() {
    let dir = tempfile::tempdir().unwrap();
    let spec = OptimizationSpec::from_yaml(&emit_optimize_artefact(dir.path())).unwrap();

    assert_eq!(spec.schema_version, "mavai-optimize-1");
    assert_eq!(spec.scorer.as_deref(), Some("observed-pass-rate"));
    assert_eq!(
        spec.convergence.total_iterations as usize,
        spec.iterations.len()
    );
    let best = &spec.iterations[spec.convergence.best_iteration as usize];
    assert!((spec.convergence.best_score - best.score).abs() < f64::EPSILON);
    assert_eq!(spec.convergence.best_factors, best.factors);
    // The built-in scorer scores by the stated observed rate.
    for iteration in &spec.iterations {
        assert!((iteration.score - iteration.statistics.observed).abs() < 1e-4);
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
        // One in six inputs fails, so the distribution names the condition,
        // and the entry counts sum to the stated failure total.
        assert_eq!(spec.statistics.failures, 1);
        let distribution = spec
            .statistics
            .failure_distribution
            .expect("failures > 0 requires the distribution");
        assert_eq!(distribution.len(), 1);
        assert_eq!(distribution[0].condition, "well-formed");
        let total: u32 = distribution.iter().map(|entry| entry.count).sum();
        assert_eq!(total, spec.statistics.failures);
    }
}

// ── artefact key discipline ─────────────────────────────────────────────────

/// The character bound the interchange key discipline places on emitted
/// mapping keys and condition identities.
const KEY_BOUND_CHARS: usize = 256;

/// A service whose violation check name embeds the full response — the
/// runtime-content case the artefact key discipline exists to bound. The
/// response echoes the input, so a long input drives a check identity far
/// past YAML's 1,024-character implicit-key limit unless the emitter bounds
/// it.
struct ContentEmbeddingService;

impl ServiceContract for ContentEmbeddingService {
    type Input = String;
    type Output = String;

    fn id(&self) -> &'static str {
        "content-embedding"
    }

    fn invoke(&self, input: &String, _cost: &mut Cost) -> Result<String, Defect> {
        Ok(input.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        Criteria::of([Criterion::meeting()
            .pass_rate(0.5)
            .name("reference-match")
            .satisfies("reference-match", |response: &String| {
                if response.len() > 64 {
                    Err(ContractViolation::new(
                        format!("reference mismatch for '{response}'"),
                        "response does not equal the reference",
                    ))
                } else {
                    Ok(())
                }
            })
            .build()])
    }
}

/// Runs an exploration whose failing samples are driven by an input longer
/// than YAML's implicit-key limit, and returns the emitted YAML.
fn emit_long_input_artefact(dir: &Path) -> String {
    let long_input = format!(
        "a very long driving input: {}\nwith a second line",
        "lorem ipsum dolor sit amet ".repeat(64)
    );
    assert!(long_input.chars().count() > 1_024);
    let inputs = vec![long_input, "short".to_string()];
    let result = ExploreExperiment::builder()
        .service_contract_id("content-embedding")
        .factors(vec![ConfigFactor { label: "config-a" }])
        .service_contract(|_: &ConfigFactor| ContentEmbeddingService)
        .samples_per_config(4)
        .inputs(&inputs)
        .experiment_id("key-discipline-run")
        .output_dir(dir)
        .build()
        .run();

    let paths = result.spec_paths().expect("output_dir was set");
    std::fs::read_to_string(&paths[0]).unwrap()
}

/// Asserts every mapping key in the document tree stays within the bound.
fn assert_keys_bounded(value: &serde_yaml::Value, path: &str) {
    match value {
        serde_yaml::Value::Mapping(mapping) => {
            for (key, nested) in mapping {
                let key_text = key.as_str().unwrap_or_default();
                assert!(
                    key_text.chars().count() <= KEY_BOUND_CHARS,
                    "mapping key of {} characters at {path} exceeds the bound",
                    key_text.chars().count()
                );
                assert_keys_bounded(nested, &format!("{path}.{key_text}"));
            }
        }
        serde_yaml::Value::Sequence(sequence) => {
            for (index, nested) in sequence.iter().enumerate() {
                assert_keys_bounded(nested, &format!("{path}[{index}]"));
            }
        }
        _ => {}
    }
}

#[test]
fn long_input_artefacts_parse_and_validate_with_bounded_keys() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = emit_long_input_artefact(dir.path());

    // The whole document — including the emitter-formatted result
    // projection — parses in a spec-strict YAML parser.
    let document: serde_yaml::Value =
        serde_yaml::from_str(&yaml).expect("emitted artefact must parse as YAML");

    // No mapping key anywhere in the document exceeds the bound.
    assert_keys_bounded(&document, "$");

    // The artefact validates against the pinned published schema.
    assert_validates("mavai-explore-1.schema.json", &yaml);

    // The failure entries carry bounded condition identities that do not
    // embed the input, distinguished by a content hash after truncation,
    // and their counts sum to the stated failures.
    let spec = ExplorationSpec::from_yaml(&yaml).unwrap();
    assert!(spec.statistics.failures > 0, "the long input must fail");
    let distribution = spec
        .statistics
        .failure_distribution
        .expect("failures > 0 requires the distribution");
    let total: u32 = distribution.iter().map(|entry| entry.count).sum();
    assert_eq!(total, spec.statistics.failures);
    for entry in &distribution {
        assert!(entry.condition.chars().count() <= KEY_BOUND_CHARS);
        assert!(!entry.condition.contains("with a second line"));
    }

    // The input itself survives, full-length, as a value in the result
    // projection — the bound governs keys and identities, not values.
    assert!(yaml.contains("lorem ipsum dolor sit amet"));
}
