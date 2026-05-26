//! Integration tests for the verdict pipeline: builder → runner → VerdictRecord.
//!
//! These tests exercise the full path from test configuration through execution
//! to verdict record construction. They verify that fields set on the builder
//! propagate correctly through the runner into the record, and that derived
//! fields (verdict_reason, baseline_provenance) are populated from real
//! execution data — not hand-built fixtures.

mod common;

use std::time::Duration;

use feotest::model::{TestIntent, ThresholdOrigin, TrialOutcome};
use feotest::ptest::ProbabilisticTestBuilder;
use feotest::ptest::builder::ThresholdApproach;
use feotest::reporting::{ConsoleRenderer, JunitXmlWriter};
use feotest::spec::SpecResolver;
use feotest::verdict::Verdict;

// ---------------------------------------------------------------------------
// Verdict reason is derived from real execution
// ---------------------------------------------------------------------------

#[test]
fn pass_verdict_has_rate_comparison_reason() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("vp-pass", &inputs, common::always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 30,
            min_pass_rate: 0.80,
        })
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass);
    // Reason should be "observed >= threshold"
    assert!(
        record.verdict_reason().contains(">="),
        "expected '>=' in verdict reason, got: {}",
        record.verdict_reason()
    );
}

#[test]
fn fail_verdict_has_rate_comparison_reason() {
    let inputs: Vec<String> = (0..10)
        .map(|i| if i < 8 { "fail".into() } else { "ok".into() })
        .collect();

    let result = ProbabilisticTestBuilder::new("vp-fail", &inputs, |input| {
        if input == "fail" {
            TrialOutcome::failure(
                feotest::model::ContractViolation::new("check", "forced"),
                Duration::from_millis(1),
            )
        } else {
            TrialOutcome::success(Duration::from_millis(1))
        }
    })
    .approach(ThresholdApproach::ThresholdFirst {
        samples: 100,
        min_pass_rate: 0.90,
    })
    .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Fail);
    assert!(
        record.verdict_reason().contains('<'),
        "expected '<' in verdict reason, got: {}",
        record.verdict_reason()
    );
}

// ---------------------------------------------------------------------------
// Baseline provenance propagation
// ---------------------------------------------------------------------------

#[test]
fn baseline_provenance_populated_from_spec() {
    let dir = common::establish_baseline("vp-baseline-prov", 200);

    let result = common::run_against_baseline(
        "vp-baseline-prov",
        dir.path(),
        50,
        0.80,
        common::always_succeeds,
    );

    let record = result.verdict_record();
    let bp = record
        .baseline_provenance()
        .expect("baseline provenance should be populated when a spec is used");

    assert_eq!(bp.source_file(), "vp-baseline-prov.yaml");
    assert!(bp.baseline_samples() > 0);
    assert!(bp.baseline_rate() > 0.0);
    assert!(bp.derived_threshold() > 0.0);
    assert!(!bp.generated_at().is_empty());
}

#[test]
fn no_baseline_provenance_without_spec() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("vp-no-bp", &inputs, common::always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 30,
            min_pass_rate: 0.80,
        })
        .run();

    assert!(
        result.verdict_record().baseline_provenance().is_none(),
        "should have no baseline provenance without a spec"
    );
}

// ---------------------------------------------------------------------------
// Covariate status defaults
// ---------------------------------------------------------------------------

#[test]
fn covariate_status_aligned_by_default() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("vp-cov-default", &inputs, common::always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 30,
            min_pass_rate: 0.80,
        })
        .run();

    let cov = result.verdict_record().covariate_status();
    assert!(cov.aligned());
    assert!(cov.misalignments().is_empty());
    assert!(cov.baseline_profile().is_empty());
    assert!(cov.observed_profile().is_empty());
}

// ---------------------------------------------------------------------------
// Spec provenance propagation
// ---------------------------------------------------------------------------

#[test]
fn spec_provenance_carries_origin_and_contract() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("vp-prov", &inputs, common::always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 50,
            min_pass_rate: 0.80,
        })
        .threshold_origin(ThresholdOrigin::Sla)
        .contract_ref("SLA v3.0 §2.1")
        .run();

    let prov = result
        .verdict_record()
        .spec_provenance()
        .expect("provenance present");
    assert_eq!(prov.threshold_origin(), ThresholdOrigin::Sla);
    assert_eq!(prov.contract_ref(), Some("SLA v3.0 §2.1"));
}

#[test]
fn spec_provenance_includes_filename_when_baseline_used() {
    let dir = common::establish_baseline("vp-prov-file", 200);

    let result = common::run_against_baseline(
        "vp-prov-file",
        dir.path(),
        50,
        0.80,
        common::always_succeeds,
    );

    let prov = result
        .verdict_record()
        .spec_provenance()
        .expect("provenance present");
    assert_eq!(prov.spec_filename(), Some("vp-prov-file.yaml"));
}

// ---------------------------------------------------------------------------
// Sample-size-first pipeline
// ---------------------------------------------------------------------------

#[test]
fn sample_size_first_derives_threshold_from_baseline() {
    let dir = common::establish_baseline("vp-ssf", 200);

    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("vp-ssf", &inputs, common::always_succeeds)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(SpecResolver::with_dir(dir.path()))
        .threshold_origin(ThresholdOrigin::Empirical)
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass);
    let stats = record
        .statistical_analysis()
        .expect("statistical analysis present");
    assert!(
        stats.threshold() > 0.0,
        "threshold should be derived from baseline"
    );
    assert!(record.baseline_provenance().is_some());
}

// ---------------------------------------------------------------------------
// Smoke intent propagation
// ---------------------------------------------------------------------------

#[test]
fn smoke_intent_propagates_and_adds_warning() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("vp-smoke", &inputs, common::always_succeeds)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 50,
            min_pass_rate: 0.80,
        })
        .intent(TestIntent::Smoke)
        .threshold_origin(ThresholdOrigin::Sla)
        .run();

    let record = result.verdict_record();
    assert_eq!(record.intent(), TestIntent::Smoke);
    assert!(
        record
            .warnings()
            .iter()
            .any(|w| w.code() == "SMOKE_NORMATIVE"),
        "smoke + SLA should produce SMOKE_NORMATIVE warning"
    );
}

// ---------------------------------------------------------------------------
// Verdict record renders through all output formats
// ---------------------------------------------------------------------------

#[test]
fn runner_produced_record_renders_through_console() {
    let dir = common::establish_baseline("vp-console", 200);

    let result =
        common::run_against_baseline("vp-console", dir.path(), 50, 0.80, common::always_succeeds);

    let renderer = ConsoleRenderer::without_colour();
    let output = renderer.render_verdict_to_string(result.verdict_record());

    assert!(output.contains("VERDICT: PASS"));
    assert!(output.contains("Pass rate:"));
    assert!(output.contains("Baseline:"));
    assert!(output.contains("vp-console.yaml"));
}

#[test]
fn runner_produced_record_renders_through_junit_xml() {
    let dir = common::establish_baseline("vp-junit", 200);

    let result =
        common::run_against_baseline("vp-junit", dir.path(), 50, 0.80, common::always_succeeds);

    let mut buf = Vec::new();
    JunitXmlWriter::write_to(&mut buf, &[result.verdict_record().clone()]).unwrap();
    let xml = String::from_utf8(buf).unwrap();

    assert!(xml.contains("testsuite"));
    assert!(xml.contains("tests=\"1\""));
    assert!(xml.contains("failures=\"0\""));
    assert!(xml.contains("vp-junit"));
}

#[test]
fn fail_record_renders_through_both_formats() {
    let inputs: Vec<String> = (0..5)
        .map(|i| if i < 4 { "fail".into() } else { "ok".into() })
        .collect();

    let result = ProbabilisticTestBuilder::new("vp-fail-render", &inputs, |input| {
        if input == "fail" {
            TrialOutcome::failure(
                feotest::model::ContractViolation::new("check", "forced"),
                Duration::from_millis(1),
            )
        } else {
            TrialOutcome::success(Duration::from_millis(1))
        }
    })
    .approach(ThresholdApproach::ThresholdFirst {
        samples: 100,
        min_pass_rate: 0.90,
    })
    .run();

    // Console
    let renderer = ConsoleRenderer::without_colour();
    let console = renderer.render_verdict_to_string(result.verdict_record());
    assert!(console.contains("VERDICT: FAIL"));

    // JUnit XML
    let mut buf = Vec::new();
    JunitXmlWriter::write_to(&mut buf, &[result.verdict_record().clone()]).unwrap();
    let xml = String::from_utf8(buf).unwrap();
    assert!(xml.contains("failures=\"1\""));
    assert!(xml.contains("<failure"));
}
