//! Integration test for verdict XML serialisation.
//!
//! Verifies structural invariants of the generated XML: expected elements,
//! namespace, self-containment, and correct handling of all optional fields.

use std::time::Duration;

use feotest::controls::PacingConfig;
use feotest::model::{
    CostSummary, ExecutionSummary, ExpirationInfo, ExpirationStatus, PacingSummary,
    TerminationInfo, TerminationReason, TestIdentity, TestIntent, ThresholdOrigin, Warning,
};
use feotest::reporting::VerdictXmlWriter;
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

fn full_record() -> VerdictRecord {
    let analysis = StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
        .with_test_results(2.294, 0.011);

    let provenance = SpecProvenance::new(ThresholdOrigin::Empirical)
        .with_spec_filename("full-service.yaml")
        .with_contract_ref("SLA v2.0 §3")
        .with_expiration(ExpirationInfo::new(
            ExpirationStatus::Valid,
            Some("2026-12-31T23:59:59Z".into()),
        ));

    let baseline_prov = BaselineProvenance::new(
        "full-service.yaml",
        "2026-03-01T08:00:00Z",
        500,
        0.9600,
        0.9200,
    );

    let pacing = PacingSummary::from_config(&PacingConfig::new().max_requests_per_second(5.0));

    VerdictRecord::builder(
        TestIdentity::new("full-service").with_test_name("test_everything"),
        Verdict::Pass,
        TestIntent::Verification,
        sample_execution(200, 200, 192, 8),
        FunctionalAssessment::single(CriterionRow::result(192, 8, vec![], Verdict::Pass)),
    )
    .statistical_analysis(analysis)
    .spec_provenance(provenance)
    .baseline_provenance(baseline_prov)
    .correlation_id("integration-run-1")
    .pacing(pacing)
    .environment(vec![
        ("runtime".to_string(), "tokio".to_string()),
        ("region".to_string(), "us-east-1".to_string()),
    ])
    .warning(Warning::new("BASELINE_AGING", "Baseline is 49 days old"))
    .build()
}

#[test]
fn full_verdict_contains_all_rp07_elements() {
    let xml = VerdictXmlWriter::write_record(&full_record(), Some("2026-04-19T10:00:00Z"));

    // Root structure. The namespace is stable across schema revisions; the
    // per-criterion bundle a contract-driven record carries lifts it to 1.2.
    assert!(xml.contains("xmlns=\"http://mavai.org/verdict/1.0\""));
    assert!(xml.contains("version=\"1.2\""));
    assert!(xml.contains("generator=\"feotest/"));

    // All verdict XML elements present
    assert!(xml.contains("<identity "));
    assert!(xml.contains("<execution "));
    assert!(xml.contains("<functional "));
    assert!(xml.contains("<statistics "));
    // <statistics> carries the one-sided Wilson lower bound — `wilson-lower` —
    // not the legacy two-sided `ci-lower` / `ci-upper` pair.
    assert!(xml.contains("wilson-lower="));
    assert!(!xml.contains("ci-lower="));
    assert!(!xml.contains("ci-upper="));
    assert!(xml.contains("<covariates "));
    assert!(xml.contains("<provenance "));
    assert!(xml.contains("<baseline "));
    assert!(xml.contains("<termination "));
    assert!(xml.contains("<cost "));
    assert!(xml.contains("<warnings>"));
    assert!(xml.contains("<pacing "));
    assert!(xml.contains("<environment>"));
    assert!(xml.contains("<per-criterion>"));
    assert!(xml.contains("<criterion "));
    assert!(xml.contains("<composite "));
    assert!(xml.contains("<verdict "));

    // New elements specific to this implementation
    assert!(xml.contains("correlation-id=\"integration-run-1\""));
    assert!(xml.contains("<expiration "));
    assert!(xml.contains("status=\"VALID\""));
    assert!(xml.contains("expires-at=\"2026-12-31T23:59:59Z\""));
    assert!(xml.contains("max-rps=\"5\""));
    assert!(xml.contains("effective-min-delay-ms=\"200\""));
    assert!(xml.contains("key=\"runtime\" value=\"tokio\""));
    assert!(xml.contains("key=\"region\" value=\"us-east-1\""));
}

#[test]
fn xml_is_self_contained() {
    let xml = VerdictXmlWriter::write_record(&full_record(), Some("2026-04-19T10:00:00Z"));

    // No external references
    assert!(!xml.contains("href="));
    assert!(!xml.contains("src="));
    assert!(!xml.contains("xlink:"));
}

#[test]
fn suite_wraps_multiple_records() {
    let record = full_record();
    let xml1 = VerdictXmlWriter::write_record(&record, Some("2026-04-19T10:00:00Z"));
    let xml2 = VerdictXmlWriter::write_record(&record, Some("2026-04-19T10:01:00Z"));
    let suite = VerdictXmlWriter::wrap_suite(&[xml1, xml2], Some("2026-04-19T10:02:00Z"));

    assert!(suite.contains("<verdict-suite"));
    assert!(suite.contains("</verdict-suite>"));
    // Should contain two verdict-record elements (XML declaration stripped)
    assert_eq!(suite.matches("<verdict-record").count(), 2);
    assert_eq!(suite.matches("</verdict-record>").count(), 2);
    // Only one XML declaration at the top
    assert_eq!(suite.matches("<?xml").count(), 1);
}
