//! Integration test for the HTML report writer.
//!
//! Verifies structural invariants of the generated HTML: valid document
//! structure, self-containment (no external resources), and expected
//! content presence.
//!
//! These tests require `xsltproc` to be installed. Tests are skipped
//! gracefully when it is not available.

use std::io;
use std::time::Duration;

use feotest::model::{
    CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity, TestIntent,
    ThresholdOrigin,
};
use feotest::reporting::HtmlReportWriter;
use feotest::verdict::{
    FunctionalDimension, SpecProvenance, StatisticalAnalysis, Verdict, VerdictRecord,
};

fn sample_execution(
    planned: u32,
    executed: u32,
    successes: u32,
    failures: u32,
) -> ExecutionSummary {
    ExecutionSummary::new(
        planned,
        executed,
        successes,
        failures,
        TerminationInfo::new(TerminationReason::Completed),
        CostSummary::new(Duration::from_millis(500), 1000, executed),
    )
}

fn pass_record() -> VerdictRecord {
    let analysis = StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
        .with_test_results(2.294, 0.011);
    let provenance =
        SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("my-service.yaml");

    VerdictRecord::builder(
        TestIdentity::new("my-service").with_test_name("test_translation"),
        Verdict::Pass,
        TestIntent::Verification,
        sample_execution(100, 100, 96, 4),
        FunctionalDimension::new(96, 4, vec![]),
    )
    .statistical_analysis(analysis)
    .spec_provenance(provenance)
    .build()
}

fn fail_record() -> VerdictRecord {
    let analysis = StatisticalAnalysis::new(0.95, 0.040, 0.722, 0.900, ThresholdOrigin::Empirical)
        .with_test_results(-1.500, 0.933);

    VerdictRecord::builder(
        TestIdentity::new("payment-service").with_test_name("test_payment_accuracy"),
        Verdict::Fail,
        TestIntent::Verification,
        sample_execution(100, 100, 80, 20),
        FunctionalDimension::new(
            80,
            20,
            vec![("parse".to_string(), 12), ("content".to_string(), 8)],
        ),
    )
    .statistical_analysis(analysis)
    .build()
}

fn inconclusive_record() -> VerdictRecord {
    VerdictRecord::builder(
        TestIdentity::new("flaky-service"),
        Verdict::Inconclusive,
        TestIntent::Verification,
        sample_execution(10, 10, 7, 3),
        FunctionalDimension::new(7, 3, vec![]),
    )
    .build()
}

/// Helper that generates HTML and skips the test if xsltproc is unavailable.
fn generate_or_skip(records: &[VerdictRecord], timestamp: Option<&str>) -> Option<String> {
    match HtmlReportWriter::generate(records, timestamp) {
        Ok(html) => Some(html),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            eprintln!("skipping: xsltproc not installed");
            None
        }
        Err(e) => panic!("unexpected error: {e}"),
    }
}

#[test]
fn report_contains_expected_structure() {
    let Some(html) = generate_or_skip(
        &[pass_record(), fail_record()],
        Some("2026-04-01T12:00:00Z"),
    ) else {
        return;
    };

    assert!(html.contains("<html"));
    assert!(html.contains("<style>"));
    assert!(html.contains("feotest Report"));
}

#[test]
fn report_contains_no_external_resources() {
    let Some(html) = generate_or_skip(
        &[pass_record(), fail_record(), inconclusive_record()],
        Some("2026-04-01T12:00:00Z"),
    ) else {
        return;
    };

    assert!(!html.contains("<script"));
    assert!(!html.contains("<img"));
}

#[test]
fn report_contains_expected_test_names() {
    let Some(html) = generate_or_skip(
        &[pass_record(), fail_record()],
        Some("2026-04-01T12:00:00Z"),
    ) else {
        return;
    };

    assert!(html.contains("test_translation"));
    assert!(html.contains("test_payment_accuracy"));
}

#[test]
fn report_contains_expected_verdicts() {
    let Some(html) = generate_or_skip(
        &[pass_record(), fail_record(), inconclusive_record()],
        Some("2026-04-01T12:00:00Z"),
    ) else {
        return;
    };

    assert!(html.contains("PASS"));
    assert!(html.contains("FAIL"));
    assert!(html.contains("INCONCLUSIVE"));
}

#[test]
fn report_contains_summary_counts() {
    let Some(html) = generate_or_skip(
        &[pass_record(), fail_record(), inconclusive_record()],
        Some("2026-04-01T12:00:00Z"),
    ) else {
        return;
    };

    assert!(html.contains("Total: 3"));
    assert!(html.contains("Pass: 1"));
    assert!(html.contains("Fail: 1"));
    assert!(html.contains("Inconclusive: 1"));
}

#[test]
fn report_contains_assumptions_section() {
    let Some(html) = generate_or_skip(&[pass_record()], Some("2026-04-01T12:00:00Z")) else {
        return;
    };

    assert!(html.contains("Statistical assumptions"));
    assert!(html.contains("Bernoulli experiment"));
}

#[test]
fn report_groups_by_service_contract() {
    let Some(html) = generate_or_skip(
        &[pass_record(), fail_record()],
        Some("2026-04-01T12:00:00Z"),
    ) else {
        return;
    };

    assert!(html.contains("my-service"));
    assert!(html.contains("payment-service"));
}

#[test]
fn empty_records_produce_valid_html() {
    let Some(html) = generate_or_skip(&[], Some("2026-04-01T12:00:00Z")) else {
        return;
    };

    assert!(html.contains("<html"));
    assert!(html.contains("Total: 0"));
}
