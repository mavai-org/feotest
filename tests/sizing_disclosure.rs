//! Sizing-transparency facts on the verdict record, end to end.
//!
//! Drives scripted contracts through the production measurement and test
//! loops and asserts the verdict record carries the run-design facts the
//! report renders: the approach with its declared parameters on every run,
//! and the detectable-rate/savings pair exactly when the run was sized
//! below its baseline's own measurement.

use std::sync::atomic::{AtomicU32, Ordering};

use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::MeasureExperiment;
use feotest::model::{ContractViolation, ThresholdOrigin};
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::service_contract::ServiceContract;
use feotest::spec::SpecResolver;
use feotest::statistics::risk_driven_sizing;
use feotest::statistics::types::ConfidenceLevel;
use feotest::verdict::VerdictRecord;

/// A single-criterion contract that passes exactly the first `passing`
/// judged samples — deterministic baseline rates through the production
/// measurement loop.
struct ScriptedContract {
    id: String,
    passing: u32,
}

impl ServiceContract for ScriptedContract {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        &self.id
    }

    fn invoke(
        &self,
        input: &String,
        _cost: &mut feotest::controls::Cost,
    ) -> Result<String, feotest::model::Defect> {
        Ok(input.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        let passing = self.passing;
        let judged = AtomicU32::new(0);
        Criteria::of([Criterion::empirical()
            .pass_rate()
            .name("accuracy")
            .satisfies("accuracy", move |_: &String| {
                if judged.fetch_add(1, Ordering::SeqCst) < passing {
                    Ok(())
                } else {
                    Err(ContractViolation::new("scripted", "scripted failure"))
                }
            })
            .build()])
    }
}

/// Establishes a baseline of `trials` samples at rate `passing`/`trials` in
/// a fresh temp directory (returned to keep it alive).
fn establish_baseline(service_contract_id: &str, passing: u32, trials: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let id = service_contract_id.to_owned();

    MeasureExperiment::builder()
        .service_contract_id(service_contract_id)
        .service_contract(move || ScriptedContract {
            id: id.clone(),
            passing,
        })
        .samples(trials)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    dir
}

/// Runs an all-passing test against the baseline under the given approach.
fn run_with_approach(
    service_contract_id: &str,
    baseline_dir: &std::path::Path,
    approach: ThresholdApproach,
) -> feotest::ptest::ProbabilisticTestResult {
    let inputs = vec!["input".to_string()];
    ProbabilisticTest::for_contract(ScriptedContract {
        id: service_contract_id.to_owned(),
        passing: u32::MAX,
    })
    .inputs(&inputs)
    .approach(approach)
    .threshold_origin(ThresholdOrigin::Empirical)
    .spec_resolver(SpecResolver::with_dir(baseline_dir))
    .run()
}

fn entry<'a>(record: &'a VerdictRecord, key: &str) -> Option<&'a str> {
    record
        .environment()
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

#[test]
fn every_run_records_its_approach_with_declared_parameters() {
    let id = "disclosure-risk-driven";
    let baseline_dir = establish_baseline(id, 96, 100);

    let result = run_with_approach(
        id,
        baseline_dir.path(),
        ThresholdApproach::RiskDriven {
            minimum_acceptable_rate: 0.93,
            confidence: 0.95,
            target_power: 0.80,
        },
    );
    let record = result.verdict_record();

    assert_eq!(
        entry(record, "sizing-approach"),
        Some("confidence-first (risk-driven)")
    );
    assert_eq!(entry(record, "sizing-tolerated-rate"), Some("0.93"));
    assert_eq!(entry(record, "sizing-declared-confidence"), Some("0.95"));
    assert_eq!(entry(record, "sizing-declared-power"), Some("0.8"));
    assert_eq!(
        entry(record, "sizing-computed-samples"),
        Some(record.execution().samples_planned().to_string().as_str())
    );
    // Risk-driven sizing prices the run above the baseline's own 100
    // samples here, so there is no downsizing trade to disclose.
    assert!(entry(record, "sizing-detectable-rate").is_none());
    assert!(entry(record, "sizing-saved-fraction").is_none());
}

#[test]
fn a_downsized_run_records_the_trade_from_the_sizing_statistics() {
    let id = "disclosure-downsized";
    let baseline_dir = establish_baseline(id, 192, 200);

    let result = run_with_approach(
        id,
        baseline_dir.path(),
        ThresholdApproach::SampleSizeFirst {
            samples: 50,
            confidence: 0.95,
        },
    );
    let record = result.verdict_record();

    assert_eq!(entry(record, "sizing-approach"), Some("sample-size-first"));
    let disclosed: f64 = entry(record, "sizing-detectable-rate")
        .expect("a run sized below its baseline must disclose the detectable rate")
        .parse()
        .unwrap();
    let expected = risk_driven_sizing::detectable_rate(50, 0.96, ConfidenceLevel::new(0.95), 0.80);
    assert!((disclosed - expected).abs() < 1e-12);
    assert_eq!(entry(record, "sizing-detectable-power"), Some("0.8"));
    assert_eq!(entry(record, "sizing-saved-fraction"), Some("0.75"));
    assert!(entry(record, "sizing-time-saved-ms").is_some());
    // The scripted contract records no token costs; the token half of the
    // efficiency disclosure degrades away rather than blocking the record.
    assert!(entry(record, "sizing-tokens-saved").is_none());
}

#[test]
fn a_full_size_run_discloses_only_its_approach() {
    let id = "disclosure-full-size";
    let baseline_dir = establish_baseline(id, 96, 100);

    let result = run_with_approach(
        id,
        baseline_dir.path(),
        ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        },
    );
    let record = result.verdict_record();

    assert_eq!(entry(record, "sizing-approach"), Some("sample-size-first"));
    assert!(entry(record, "sizing-detectable-rate").is_none());
    assert!(entry(record, "sizing-saved-fraction").is_none());
}
