//! Emitter conformance for the verdict XML interchange.
//!
//! The verdict XML this crate emits is validated against the vendored copy
//! of the published family schema
//! (`tests/conformance/interchange/verdict-1.7.xsd`, pinned per family
//! schema release) — not merely against this crate's own snapshots, which
//! could drift together with the emitter. The records validated come from
//! the production run path wherever the shape can be produced by a run: a
//! decided record naming its rules, a record with a saturated latency
//! evaluation, a record with an explicit latency requirement, and a refused
//! configuration. The co-constraints XSD 1.0 cannot state (a refused record
//! states codes and no value; a saturated evaluation has no threshold and no
//! baseline rank) are asserted here. Validation shells out to `xmllint`;
//! when it is not installed the test skips, mirroring the HTML report tests'
//! handling of `xsltproc`.

use std::io::Write as _;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::MeasureExperiment;
use feotest::latency::{LatencyCriterion, Percentile};
use feotest::model::{
    ContractViolation, CostSummary, ExecutionSummary, TerminationInfo, TerminationReason,
    TestIdentity, TestIntent, ThresholdOrigin,
};
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::reporting::VerdictXmlWriter;
use feotest::service_contract::ServiceContract;
use feotest::spec::SpecResolver;
use feotest::verdict::{
    CriterionRow, FunctionalAssessment, SpecProvenance, Verdict, VerdictRecord,
};

mod common;

/// The vendored published schema.
const XSD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/conformance/interchange/verdict-1.7.xsd"
);

/// Validates a record against the published schema, returning the record's
/// XML; skips the schema check when `xmllint` is not installed.
fn assert_validates(record: &VerdictRecord) -> String {
    let record_xml = VerdictXmlWriter::write_record(record, Some("2026-09-28T00:00:00Z"));
    let xml = VerdictXmlWriter::wrap_suite(
        std::slice::from_ref(&record_xml),
        Some("2026-09-28T00:00:00Z"),
    );

    let mut xml_file = tempfile::NamedTempFile::new().unwrap();
    xml_file.write_all(xml.as_bytes()).unwrap();

    let output = match Command::new("xmllint")
        .args(["--noout", "--schema", XSD])
        .arg(xml_file.path())
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping: xmllint not installed");
            return record_xml;
        }
        Err(error) => panic!("failed to run xmllint: {error}"),
    };

    assert!(
        output.status.success(),
        "emitted verdict XML violates the published schema:\n{}\n{xml}",
        String::from_utf8_lossy(&output.stderr)
    );
    record_xml
}

/// A contract whose criteria pass on exactly the first `passing` judged
/// samples: a baseline-derived criterion, and optionally a requirement over
/// the same postcondition, decided at its own confidence.
struct ScriptedContract {
    id: &'static str,
    passing: u32,
    requirement: Option<(f64, f64)>,
    p95_ceiling: Option<Duration>,
}

impl ScriptedContract {
    const fn all_passing(id: &'static str) -> Self {
        Self {
            id,
            passing: u32::MAX,
            requirement: None,
            p95_ceiling: None,
        }
    }
}

/// A postcondition that holds on exactly the first `passing` judged samples.
fn scripted(passing: u32) -> impl Fn(&String) -> feotest::model::Outcome + Send + Sync {
    let judged = AtomicU32::new(0);
    move |_: &String| {
        if judged.fetch_add(1, Ordering::SeqCst) < passing {
            Ok(())
        } else {
            Err(ContractViolation::new("well-formed", "scripted failure"))
        }
    }
}

impl ServiceContract for ScriptedContract {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        self.id
    }

    fn invoke(
        &self,
        input: &String,
        _cost: &mut feotest::controls::Cost,
    ) -> Result<String, feotest::model::Defect> {
        Ok(input.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        let regression = Criterion::empirical()
            .pass_rate()
            .name("well-formed-regression")
            .satisfies("well-formed", scripted(self.passing))
            .build();
        let Some((rate, confidence)) = self.requirement else {
            return Criteria::of([regression]);
        };
        let compliance = Criterion::meeting()
            .pass_rate(rate)
            .name("well-formed-compliance")
            .confidence(confidence)
            .satisfies("well-formed", scripted(self.passing))
            .build();
        Criteria::of([compliance, regression])
    }

    fn latency(&self) -> Option<LatencyCriterion> {
        self.p95_ceiling
            .map(|ceiling| LatencyCriterion::meeting().at_most(Percentile::P95, ceiling))
    }
}

/// Measures a baseline of `trials` samples, all passing, in a fresh
/// directory (returned to keep it alive).
fn establish_baseline(id: &'static str, trials: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    MeasureExperiment::builder()
        .service_contract_id(id)
        .service_contract(move || ScriptedContract::all_passing(id))
        .samples(trials)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();
    dir
}

/// Runs a scripted contract against its baseline, every planned sample.
fn run(
    contract: ScriptedContract,
    baseline: &tempfile::TempDir,
    samples: u32,
    strict_latency: bool,
) -> VerdictRecord {
    let inputs = vec!["input".to_string()];
    ProbabilisticTest::for_contract(contract)
        .inputs(&inputs)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples,
            confidence: 0.95,
        })
        .threshold_origin(ThresholdOrigin::Sla)
        .spec_resolver(SpecResolver::with_dir(baseline.path()))
        .enforce_baseline_latency(strict_latency)
        .disable_early_termination()
        .run()
        .verdict_record()
        .clone()
}

#[test]
fn a_decided_record_naming_its_rules_validates() {
    // A requirement of 0.90 at alpha 0.01 and a baseline of 1000 over the
    // same postcondition: two criteria, two rules.
    let baseline = establish_baseline("xsd-two-criteria", 1000);
    let record = run(
        ScriptedContract {
            passing: 93,
            requirement: Some((0.90, 0.99)),
            ..ScriptedContract::all_passing("xsd-two-criteria")
        },
        &baseline,
        100,
        false,
    );
    assert!(!record.is_refused());
    let xml = assert_validates(&record);
    assert!(xml.contains("methodology-version=\"1.5.0\""));
    assert!(xml.contains("decision-rule=\"compliance/exact-binomial\""));
    assert!(xml.contains("decision-rule=\"regression/fisher\""));
}

#[test]
fn a_record_with_a_saturated_latency_evaluation_validates() {
    // Twenty latencies against a baseline of 100: no baseline rank keeps
    // the no-degradation breach probability of p99 at or below alpha.
    let baseline = establish_baseline("xsd-saturated", 100);
    let record = run(
        ScriptedContract::all_passing("xsd-saturated"),
        &baseline,
        20,
        true,
    );
    let latency = record
        .latency()
        .expect("baseline latencies give a latency dimension");
    assert_eq!(latency.verdict(), Some(Verdict::Inconclusive));
    assert_eq!(record.verdict(), Some(Verdict::Inconclusive));

    let xml = assert_validates(&record);
    let saturated: Vec<&str> = xml
        .lines()
        .filter(|line| line.contains("status=\"SATURATED\""))
        .collect();
    assert!(!saturated.is_empty(), "a saturated evaluation is emitted");
    for line in saturated {
        assert!(!line.contains("threshold-ms"), "saturated: no threshold");
        assert!(!line.contains("baseline-rank"), "saturated: no rank");
    }
}

#[test]
fn a_record_with_an_explicit_latency_requirement_validates() {
    let baseline = establish_baseline("xsd-explicit-latency", 100);
    let record = run(
        ScriptedContract {
            p95_ceiling: Some(Duration::from_secs(5)),
            ..ScriptedContract::all_passing("xsd-explicit-latency")
        },
        &baseline,
        60,
        false,
    );
    let xml = assert_validates(&record);
    assert!(xml.contains("decision-rule=\"latency/compliance-exact-binomial\""));
    assert!(xml.contains("within-threshold=\"60\""));
    assert!(xml.contains("required-within="));
}

#[test]
fn a_configuration_refused_on_both_parts_validates() {
    // The test (200) is larger than its baseline (100), and no count of 200
    // can demonstrate 0.999 at alpha 0.05: both codes, in the fixed order.
    let baseline = establish_baseline("xsd-refused", 100);
    let record = run(
        ScriptedContract {
            requirement: Some((0.999, 0.95)),
            ..ScriptedContract::all_passing("xsd-refused")
        },
        &baseline,
        200,
        false,
    );
    assert!(record.is_refused());
    assert_eq!(record.execution().samples_executed(), 0);

    let xml = assert_validates(&record);
    assert!(xml.contains(
        "<verdict configuration-error=\"TEST_LARGER_THAN_BASELINE COMPLIANCE_INFEASIBLE\""
    ));
    assert!(!xml.contains("<verdict value="));
    assert!(xml.contains("<termination reason=\"CONFIGURATION_REFUSED\""));
}

#[test]
fn a_hand_assembled_record_validates() {
    let execution = ExecutionSummary::new(
        100,
        100,
        97,
        3,
        TerminationInfo::new(TerminationReason::Completed),
        CostSummary::new(Duration::from_millis(500), 1000, 100),
    );
    let analysis = common::regression_analysis(97, 100, 951, 1000);
    let provenance =
        SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("conformance.yaml");

    let record = VerdictRecord::builder(
        TestIdentity::new("conformance-service").with_test_name("verdict_emitter"),
        Verdict::Pass,
        TestIntent::Verification,
        execution,
        FunctionalAssessment::single(CriterionRow::new(
            "well-formed",
            97,
            3,
            vec![("well-formed".to_string(), 3)],
            Some(analysis.clone()),
            Verdict::Pass,
        )),
    )
    .statistical_analysis(analysis)
    .spec_provenance(provenance)
    .build();

    assert_validates(&record);
}
