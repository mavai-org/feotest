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
    BaselineProvenance, CriterionRow, FunctionalAssessment, SpecProvenance, StatisticalAnalysis,
    Verdict, VerdictRecord,
};

const fn sample_execution(
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
        FunctionalAssessment::single(CriterionRow::result(96, 4, vec![], Verdict::Pass)),
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
        FunctionalAssessment::single(CriterionRow::result(
            80,
            20,
            vec![("parse".to_string(), 12), ("content".to_string(), 8)],
            Verdict::Fail,
        )),
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
        FunctionalAssessment::single(CriterionRow::result(7, 3, vec![], Verdict::Inconclusive)),
    )
    .build()
}

/// A record carrying the run-design facts of a downsized risk-driven run:
/// sized at 100 samples against a baseline measured over 1,000, with both
/// cost halves recorded.
fn downsized_record(sizing_entries: Vec<(&str, &str)>) -> VerdictRecord {
    let analysis = StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical);
    VerdictRecord::builder(
        TestIdentity::new("sized-service").with_test_name("test_sized"),
        Verdict::Pass,
        TestIntent::Verification,
        sample_execution(100, 100, 96, 4),
        FunctionalAssessment::single(CriterionRow::result(96, 4, vec![], Verdict::Pass)),
    )
    .statistical_analysis(analysis)
    .baseline_provenance(BaselineProvenance::new(
        "sized-service.yaml",
        "2026-07-01T00:00:00Z",
        1000,
        0.96,
        0.9,
    ))
    .environment(
        sizing_entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
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
fn run_design_block_discloses_approach_and_sizing_trade() {
    let record = downsized_record(vec![
        ("sizing-approach", "confidence-first (risk-driven)"),
        ("sizing-tolerated-rate", "0.93"),
        ("sizing-declared-confidence", "0.95"),
        ("sizing-declared-power", "0.8"),
        ("sizing-computed-samples", "100"),
        ("sizing-detectable-rate", "0.876"),
        ("sizing-detectable-power", "0.8"),
        ("sizing-saved-fraction", "0.9"),
        ("sizing-time-saved-ms", "45000"),
        ("sizing-tokens-saved", "1080000"),
    ]);
    let Some(html) = generate_or_skip(&[record], Some("2026-07-13T12:00:00Z")) else {
        return;
    };

    assert!(html.contains("Run design"));
    assert!(html.contains("confidence-first (risk-driven)"));
    assert!(html.contains("priced against the acceptance bar"));
    assert!(html.contains("Tolerated rate"));
    assert!(html.contains("93%"));
    assert!(html.contains("Target power"));
    assert!(
        html.contains("This test was sized at 100 samples against a baseline measured over 1000.")
    );
    assert!(html.contains("would only catch a drop below 88% four times out of five."));
    assert!(html.contains("about 90% less execution time and tokens"));
    assert!(html.contains("roughly 45.0 seconds and 1080000 tokens"));
    assert!(html.contains("Estimates only."));
}

#[test]
fn efficiency_disclosure_degrades_to_time_only_without_token_costs() {
    let record = downsized_record(vec![
        ("sizing-approach", "sample-size-first"),
        ("sizing-declared-samples", "100"),
        ("sizing-declared-confidence", "0.95"),
        ("sizing-detectable-rate", "0.876"),
        ("sizing-detectable-power", "0.8"),
        ("sizing-saved-fraction", "0.9"),
        ("sizing-time-saved-ms", "45000"),
    ]);
    let Some(html) = generate_or_skip(&[record], Some("2026-07-13T12:00:00Z")) else {
        return;
    };

    assert!(html.contains("about 90% less execution time (roughly 45.0 seconds"));
    assert!(html.contains("no token figures are recorded for this run."));
    assert!(!html.contains("fewer tokens"));
}

#[test]
fn approach_disclosure_stands_alone_on_a_full_size_run() {
    let record = downsized_record(vec![
        ("sizing-approach", "sample-size-first"),
        ("sizing-declared-samples", "100"),
        ("sizing-declared-confidence", "0.95"),
    ]);
    let Some(html) = generate_or_skip(&[record], Some("2026-07-13T12:00:00Z")) else {
        return;
    };

    assert!(html.contains("Run design"));
    assert!(html.contains("sample-size-first"));
    assert!(!html.contains("would only catch a drop below"));
    assert!(!html.contains("Estimated saving"));
}

#[test]
fn records_without_sizing_facts_render_no_run_design_block() {
    let Some(html) = generate_or_skip(&[pass_record()], Some("2026-07-13T12:00:00Z")) else {
        return;
    };

    assert!(!html.contains("Run design"));
}

#[test]
fn empty_records_produce_valid_html() {
    let Some(html) = generate_or_skip(&[], Some("2026-04-01T12:00:00Z")) else {
        return;
    };

    assert!(html.contains("<html"));
    assert!(html.contains("Total: 0"));
}
