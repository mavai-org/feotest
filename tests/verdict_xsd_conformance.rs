//! Emitter conformance for the verdict XML interchange.
//!
//! The verdict XML this crate emits is validated against the vendored copy
//! of the published family schema
//! (`tests/conformance/interchange/verdict-1.2.xsd`, pinned per family
//! schema release) — not merely against this crate's own snapshots, which
//! could drift together with the emitter. Validation shells out to
//! `xmllint`; when it is not installed the test skips, mirroring the HTML
//! report tests' handling of `xsltproc`.

use std::io::Write as _;
use std::process::Command;
use std::time::Duration;

use feotest::model::{
    CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity, TestIntent,
    ThresholdOrigin,
};
use feotest::reporting::VerdictXmlWriter;
use feotest::verdict::{
    CriterionRow, FunctionalAssessment, SpecProvenance, StatisticalAnalysis, Verdict, VerdictRecord,
};

/// A representative passing record with a failure distribution and full
/// statistical analysis (the 1.2 document shape).
fn record() -> VerdictRecord {
    let execution = ExecutionSummary::new(
        100,
        100,
        96,
        4,
        TerminationInfo::new(TerminationReason::Completed),
        CostSummary::new(Duration::from_millis(500), 1000, 100),
    );
    let analysis = StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
        .with_test_results(2.294, 0.011);
    let provenance =
        SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("conformance.yaml");

    VerdictRecord::builder(
        TestIdentity::new("conformance-service").with_test_name("verdict_emitter"),
        Verdict::Pass,
        TestIntent::Verification,
        execution,
        FunctionalAssessment::single(CriterionRow::result(
            96,
            4,
            vec![("well-formed".to_string(), 4)],
            Verdict::Pass,
        )),
    )
    .statistical_analysis(analysis)
    .spec_provenance(provenance)
    .build()
}

#[test]
fn emitted_verdict_xml_validates_against_the_published_schema() {
    let record_xml = VerdictXmlWriter::write_record(&record(), Some("2026-07-15T00:00:00Z"));
    let xml = VerdictXmlWriter::wrap_suite(&[record_xml], Some("2026-07-15T00:00:00Z"));

    let mut xml_file = tempfile::NamedTempFile::new().unwrap();
    xml_file.write_all(xml.as_bytes()).unwrap();

    let xsd = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/conformance/interchange/verdict-1.2.xsd"
    );
    let output = match Command::new("xmllint")
        .args(["--noout", "--schema", xsd])
        .arg(xml_file.path())
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping: xmllint not installed");
            return;
        }
        Err(error) => panic!("failed to run xmllint: {error}"),
    };

    assert!(
        output.status.success(),
        "emitted verdict XML violates the published schema:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
