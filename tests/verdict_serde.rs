//! Snapshot tests that pin the JSON wire shape of `VerdictRecord`.
//!
//! The shape is the contract between the sentinel runtime and any sink
//! that consumes verdict records as JSON (the built-in file and webhook
//! sinks, plus any downstream tooling). Changing a field name or omitting
//! a conditionally-present field must show up here as a snapshot diff
//! rather than silently breaking downstream consumers.

use std::time::Duration;

use feotest::model::{
    CostSummary, ExecutionSummary, ExpirationInfo, ExpirationStatus, PacingSummary,
    TerminationInfo, TerminationReason, TestIdentity, TestIntent, ThresholdOrigin, Warning,
};
use feotest::verdict::{
    BaselineProvenance, CovariateStatus, CriterionRow, FunctionalAssessment, Misalignment,
    SpecProvenance, StatisticalAnalysis, Verdict, VerdictRecord,
};

const fn sample_execution() -> ExecutionSummary {
    ExecutionSummary::new(
        100,
        100,
        95,
        5,
        TerminationInfo::new(TerminationReason::Completed),
        CostSummary::new(Duration::from_millis(500), 1234, 100),
    )
}

#[test]
fn pass_verdict_minimal() {
    let record = VerdictRecord::builder(
        TestIdentity::new("basket.translates"),
        Verdict::Pass,
        TestIntent::Verification,
        sample_execution(),
        FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
    )
    .build();

    insta::assert_json_snapshot!("pass_minimal", serde_json::to_value(&record).unwrap());
}

#[test]
fn fail_verdict_with_distribution_and_warnings() {
    let record = VerdictRecord::builder(
        TestIdentity::new("basket").with_test_name("translates"),
        Verdict::Fail,
        TestIntent::Verification,
        sample_execution(),
        FunctionalAssessment::single(CriterionRow::result(
            80,
            20,
            vec![("parse".into(), 12), ("content".into(), 8)],
            Verdict::Fail,
        )),
    )
    .statistical_analysis(
        StatisticalAnalysis::new(0.95, 0.04, 0.722, 0.90, ThresholdOrigin::Empirical)
            .with_test_results(2.29, 0.011),
    )
    .warning(Warning::new("BASELINE_EXPIRED", "Baseline is 45 days old"))
    .build();

    insta::assert_json_snapshot!(
        "fail_with_distribution",
        serde_json::to_value(&record).unwrap()
    );
}

#[test]
fn inconclusive_verdict_with_covariate_misalignment() {
    let record = VerdictRecord::builder(
        TestIdentity::new("classifier").with_test_name("accurate_enough"),
        Verdict::Inconclusive,
        TestIntent::Verification,
        sample_execution(),
        FunctionalAssessment::single(CriterionRow::result(85, 15, vec![], Verdict::Inconclusive)),
    )
    .covariate_status(CovariateStatus::new(
        false,
        vec![Misalignment::new("model", "gpt-4o", "gpt-3.5")],
        vec![("model".into(), "gpt-4o".into())],
        vec![("model".into(), "gpt-3.5".into())],
    ))
    .build();

    insta::assert_json_snapshot!(
        "inconclusive_covariate_misalignment",
        serde_json::to_value(&record).unwrap()
    );
}

#[test]
fn pass_verdict_with_baseline_provenance_and_spec_provenance() {
    let record = VerdictRecord::builder(
        TestIdentity::new("service").with_test_name("meets_sla"),
        Verdict::Pass,
        TestIntent::Verification,
        sample_execution(),
        FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
    )
    .statistical_analysis(StatisticalAnalysis::new(
        0.95,
        0.022,
        0.907,
        0.90,
        ThresholdOrigin::Empirical,
    ))
    .spec_provenance(
        SpecProvenance::new(ThresholdOrigin::Empirical)
            .with_spec_filename("service.yaml")
            .with_contract_ref("Baseline v1")
            .with_expiration(ExpirationInfo::new(
                ExpirationStatus::ExpiringSoon,
                Some("2026-06-01T00:00:00Z".into()),
            )),
    )
    .baseline_provenance(BaselineProvenance::new(
        "service.yaml",
        "2026-03-01T12:00:00Z",
        200,
        0.95,
        0.90,
    ))
    .correlation_id("run-abc")
    .environment(vec![("region".into(), "eu-west-1".into())])
    .build();

    insta::assert_json_snapshot!(
        "pass_with_baseline_provenance",
        serde_json::to_value(&record).unwrap()
    );
}

#[test]
fn verdict_with_pacing_summary() {
    use feotest::controls::PacingConfig;
    let pacing = PacingSummary::from_config(&PacingConfig::new().max_requests_per_second(10.0));

    let record = VerdictRecord::builder(
        TestIdentity::new("service").with_test_name("paced"),
        Verdict::Pass,
        TestIntent::Verification,
        sample_execution(),
        FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
    )
    .pacing(pacing)
    .build();

    insta::assert_json_snapshot!("pass_with_pacing", serde_json::to_value(&record).unwrap());
}
