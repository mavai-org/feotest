//! Emitter conformance for the verdict XML interchange.
//!
//! The verdict XML this crate emits is validated against the vendored copy
//! of the published family schema
//! (`tests/conformance/interchange/verdict-1.8.xsd`, pinned per family
//! schema release) — not merely against this crate's own snapshots, which
//! could drift together with the emitter. The records validated come from
//! the production run path wherever the shape can be produced by a run: a
//! decided record naming its rules, a record with a saturated latency
//! evaluation, a record with an explicit latency requirement, and a refused
//! configuration; a record whose functional dimension is advisory is
//! assembled by hand, the switch being the run's environment. The
//! co-constraints XSD 1.0 cannot state (a refused record states codes and no
//! value; a saturated evaluation has no threshold and no baseline rank; the
//! latency dimension states its mode exactly when it states its verdict; a
//! criterion row's `required-pass` is the count its rule decided with,
//! absent when no count can pass) are asserted here. Validation shells out
//! to `xmllint`; when it is not installed the test skips, mirroring the HTML
//! report tests' handling of `xsltproc`.

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
    CriterionRow, EnforcementMode, FunctionalAssessment, RuleEvidence, SpecProvenance, Verdict,
    VerdictRecord,
};

mod common;

/// The vendored published schema.
const XSD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/conformance/interchange/verdict-1.8.xsd"
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

/// The value of `attribute` on the `<criterion>` row with id `id`, if the
/// row carries it.
fn criterion_attribute(xml: &str, id: &str, attribute: &str) -> Option<String> {
    let row = xml
        .lines()
        .find(|line| line.contains("<criterion ") && line.contains(&format!(" id=\"{id}\"")))
        .unwrap_or_else(|| panic!("no criterion row {id} in:\n{xml}"));
    let marker = format!(" {attribute}=\"");
    let start = row.find(&marker)? + marker.len();
    let end = row[start..].find('"').unwrap() + start;
    Some(row[start..end].to_owned())
}

/// Every decided criterion row states as `required-pass` exactly the count
/// its rule decided with — the Fisher cutoff or `k_min` from the engine's
/// evidence, never a count derived from the threshold rate — and the row's
/// verdict is PASS iff its success count reaches it. A row no count can
/// pass carries no `required-pass`. Returns how many rows stated one.
fn assert_required_pass_is_the_rules_count(record: &VerdictRecord, xml: &str) -> usize {
    let mut stated = 0;
    for row in record.functional_assessment().criteria() {
        let emitted = criterion_attribute(xml, row.name(), "required-pass")
            .map(|value| value.parse::<u32>().unwrap());
        let Some(analysis) = row.statistical_analysis() else {
            assert_eq!(emitted, None, "{}: no rule, no required-pass", row.name());
            continue;
        };
        let decided_with = match analysis.evidence() {
            RuleEvidence::Regression(evidence) => Some(evidence.cutoff),
            RuleEvidence::Compliance(evidence) => evidence.minimum_passing_count,
        };
        assert_eq!(
            emitted,
            decided_with,
            "{}: the rule's own count",
            row.name()
        );
        match emitted {
            Some(required) => {
                stated += 1;
                assert_eq!(
                    row.verdict() == Verdict::Pass,
                    row.pass() >= required,
                    "{}: PASS iff pass >= required-pass",
                    row.name()
                );
            }
            None => assert_eq!(
                row.verdict(),
                Verdict::Fail,
                "{}: no count can pass",
                row.name()
            ),
        }
    }
    stated
}

/// A contract whose criteria pass on exactly the first `passing` judged
/// samples: a baseline-derived criterion, and optionally a requirement over
/// the same postcondition, decided at its own confidence.
struct ScriptedContract {
    id: &'static str,
    passing: u32,
    requirement: Option<(f64, f64)>,
    p95_ceiling: Option<Duration>,
    p99_against_baseline: bool,
}

impl ScriptedContract {
    const fn all_passing(id: &'static str) -> Self {
        Self {
            id,
            passing: u32::MAX,
            requirement: None,
            p95_ceiling: None,
            p99_against_baseline: false,
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
        if self.p99_against_baseline {
            return Some(LatencyCriterion::empirical().against_baseline(Percentile::P99));
        }
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
fn run(contract: ScriptedContract, baseline: &tempfile::TempDir, samples: u32) -> VerdictRecord {
    let inputs = vec!["input".to_string()];
    ProbabilisticTest::for_contract(contract)
        .inputs(&inputs)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples,
            confidence: 0.95,
        })
        .threshold_origin(ThresholdOrigin::Sla)
        .spec_resolver(SpecResolver::with_dir(baseline.path()))
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
    );
    assert!(!record.is_refused());
    let xml = assert_validates(&record);
    assert!(xml.contains("version=\"1.8\""));
    assert!(xml.contains("methodology-version=\"1.6.0\""));
    assert!(xml.contains("mode=\"enforced\"/>"));
    assert!(xml.contains("decision-rule=\"compliance/exact-binomial\""));
    assert!(xml.contains("decision-rule=\"regression/fisher\""));

    // Both rows state the count their rule decided with: k_min 97 for the
    // requirement (the v0.11.2 two-criteria worked example's figure), the
    // Fisher cutoff for the baseline.
    assert_eq!(assert_required_pass_is_the_rules_count(&record, &xml), 2);
    assert_eq!(
        criterion_attribute(&xml, "well-formed-compliance", "required-pass").as_deref(),
        Some("97")
    );
    assert!(criterion_attribute(&xml, "well-formed-regression", "required-pass").is_some());
}

#[test]
fn a_criterion_no_count_can_pass_states_no_required_pass() {
    // Under smoke intent an infeasible requirement runs: no count of 20 can
    // demonstrate 0.999 at alpha 0.05, so the compliance row carries no
    // required-pass, while the regression row beside it still does.
    let baseline = establish_baseline("xsd-no-count", 100);
    let inputs = vec!["input".to_string()];
    let record = ProbabilisticTest::for_contract(ScriptedContract {
        requirement: Some((0.999, 0.95)),
        ..ScriptedContract::all_passing("xsd-no-count")
    })
    .inputs(&inputs)
    .approach(ThresholdApproach::SampleSizeFirst {
        samples: 20,
        confidence: 0.95,
    })
    .smoke()
    .threshold_origin(ThresholdOrigin::Sla)
    .spec_resolver(SpecResolver::with_dir(baseline.path()))
    .disable_early_termination()
    .run()
    .verdict_record()
    .clone();
    assert!(!record.is_refused());

    let xml = assert_validates(&record);
    assert_eq!(assert_required_pass_is_the_rules_count(&record, &xml), 1);
    assert_eq!(
        criterion_attribute(&xml, "well-formed-compliance", "required-pass"),
        None
    );
    assert!(
        criterion_attribute(&xml, "well-formed-compliance", "decision-rule").is_some(),
        "the rule decided the row; only the count is absent"
    );
    assert!(criterion_attribute(&xml, "well-formed-regression", "required-pass").is_some());
}

#[test]
fn a_record_with_a_saturated_latency_evaluation_validates() {
    // Twenty latencies against a baseline of 100: no baseline rank keeps
    // the no-degradation breach probability of p99 at or below alpha.
    let baseline = establish_baseline("xsd-saturated", 100);
    let record = run(
        ScriptedContract {
            p99_against_baseline: true,
            ..ScriptedContract::all_passing("xsd-saturated")
        },
        &baseline,
        20,
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
    assert!(xml.contains("verdict=\"INCONCLUSIVE\" mode=\"enforced\">"));
    for line in saturated {
        assert!(!line.contains("threshold-ms"), "saturated: no threshold");
        assert!(!line.contains("baseline-rank"), "saturated: no rank");
    }
    // The functional criterion was still decided by its rule.
    assert_eq!(assert_required_pass_is_the_rules_count(&record, &xml), 1);
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

#[test]
fn a_record_with_an_advisory_functional_dimension_validates() {
    // The run made the functional dimension advisory: its FAIL is decided
    // and reported on the composite, and the test verdict, composed over no
    // enforced dimension, is PASS.
    let execution = ExecutionSummary::new(
        100,
        100,
        90,
        10,
        TerminationInfo::new(TerminationReason::Completed),
        CostSummary::new(Duration::from_millis(500), 1000, 100),
    );
    let analysis = common::regression_analysis(90, 100, 951, 1000);
    let record = VerdictRecord::builder(
        TestIdentity::new("conformance-service"),
        Verdict::Pass,
        TestIntent::Verification,
        execution,
        FunctionalAssessment::single(CriterionRow::new(
            "well-formed",
            90,
            10,
            vec![],
            Some(analysis.clone()),
            Verdict::Fail,
        ))
        .with_mode(EnforcementMode::Advisory),
    )
    .statistical_analysis(analysis)
    .build();

    let xml = assert_validates(&record);
    assert!(xml.contains("<composite value=\"FAIL\" mode=\"advisory\"/>"));
    assert!(xml.contains("<verdict value=\"PASS\" reason="));
    assert!(xml.contains("functional advisory"));
}
