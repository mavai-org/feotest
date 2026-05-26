//! Integration tests for the latency dimension.
//!
//! Covers the acceptance scenarios enumerated in
//! `plan/DES-LATENCY.md::Acceptance tests`.

use std::time::Duration;

use feotest::latency::{EvaluationStatus, LatencyEnforcementMode};
use feotest::model::TrialOutcome;
use feotest::ptest::ProbabilisticTestBuilder;
use feotest::ptest::builder::ThresholdApproach;
use feotest::verdict::Verdict;

/// Returns a deterministic trial closure that reports a fixed latency on
/// every call.
fn fixed_latency_trial(latency: Duration) -> impl Fn(&str) -> TrialOutcome {
    move |_input: &str| TrialOutcome::success(latency)
}

fn threshold_first(samples: u32, pass_rate: f64) -> ThresholdApproach {
    ThresholdApproach::ThresholdFirst {
        samples,
        min_pass_rate: pass_rate,
    }
}

/// A single always-pass criterion for fixtures that exercise the latency path
/// rather than response judging.
fn trivial_criteria() -> feotest::criteria::Criteria<String> {
    feotest::criteria::Criteria::of([feotest::criteria::Criteria::meeting()
        .pass_rate(0.5)
        .name("response received")
        .satisfies("response received", |_: &String| Ok(()))
        .build()])
}

/// A contract that sleeps a fixed duration on every invocation, so the engine
/// measures that latency for the baseline. (Latency is measured from real
/// invoke-elapsed; there is no synthetic-latency seam.)
struct SleepingContract {
    latency: Duration,
}

impl feotest::service_contract::ServiceContract for SleepingContract {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        "latency-baseline"
    }

    fn invoke(
        &self,
        input: &String,
        _cost: &mut feotest::controls::Cost,
    ) -> Result<String, feotest::model::Defect> {
        std::thread::sleep(self.latency);
        Ok(input.clone())
    }

    fn criteria(&self) -> feotest::criteria::Criteria<String> {
        trivial_criteria()
    }
}

// Scenario 1 — no latency config → dimension absent, assert_all ≡ assert_contract.
#[test]
fn scenario_no_latency_config_dimension_absent() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new(
        "latency-scenario-1",
        &inputs,
        fixed_latency_trial(Duration::from_millis(10)),
    )
    .approach(threshold_first(30, 0.80))
    .run();

    assert!(result.verdict_record().latency().is_none());
    assert!(result.passed());
    result.verdict_record().assert_all();
}

// Scenario 2 — explicit p95 met → dimension present, zero violations, pass.
#[test]
fn scenario_explicit_p95_met() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new(
        "latency-scenario-2",
        &inputs,
        fixed_latency_trial(Duration::from_millis(10)),
    )
    .approach(threshold_first(30, 0.80))
    .latency_p95(Duration::from_millis(50))
    .run();

    let record = result.verdict_record();
    let dim = record.latency().expect("latency dimension present");
    assert_eq!(dim.strict_violations(), 0);
    assert_eq!(dim.evaluations().len(), 1);
    assert_eq!(dim.evaluations()[0].status(), EvaluationStatus::Pass);
    assert!(result.passed());
    record.assert_all();
}

// Scenario 3 — explicit p95 violated → assert_latency panics, assert_contract ok.
#[test]
fn scenario_explicit_p95_violated_overall_fail() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new(
        "latency-scenario-3",
        &inputs,
        fixed_latency_trial(Duration::from_millis(500)),
    )
    .approach(threshold_first(30, 0.80))
    .latency_p95(Duration::from_millis(50))
    .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass, "functional still passes");
    assert!(!result.passed(), "overall must fail due to latency");
    let dim = record.latency().unwrap();
    assert_eq!(dim.strict_violations(), 1);
    record.assert_contract(); // functional ok
}

#[test]
#[should_panic(expected = "latency contract failed")]
fn scenario_explicit_p95_violated_assert_latency_panics() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new(
        "latency-scenario-3b",
        &inputs,
        fixed_latency_trial(Duration::from_millis(500)),
    )
    .approach(threshold_first(30, 0.80))
    .latency_p95(Duration::from_millis(50))
    .run();
    result.verdict_record().assert_latency();
}

// Scenarios 4 & 5 — baseline-derived p95 violated, advisory (default) vs strict.
//
// Scenario 6 (env-var strict enforcement) is exercised via a direct unit
// test in `src/latency/enforcement.rs`; integration tests avoid mutating
// process-wide environment variables so that they remain parallel-safe.
fn build_baseline_and_run(
    test_name: &str,
    baseline_latency: Duration,
    test_latency: Duration,
    strict: Option<bool>,
) -> feotest::ptest::ProbabilisticTestResult {
    let dir = tempfile::tempdir().unwrap();
    let uc_id: &'static str = Box::leak(test_name.to_string().into_boxed_str());
    let inputs = vec!["input".to_string()];

    // Establish baseline with low-latency samples (the contract sleeps so the
    // engine measures a real latency).
    feotest::experiment::MeasureExperiment::builder()
        .service_contract_id(uc_id)
        .service_contract(move || SleepingContract { latency: baseline_latency })
        .samples(150)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = feotest::spec::SpecResolver::with_dir(dir.path());
    let mut b = ProbabilisticTestBuilder::new(uc_id, &inputs, fixed_latency_trial(test_latency))
        .approach(threshold_first(30, 0.80))
        .threshold_origin(feotest::model::ThresholdOrigin::Sla)
        .spec_resolver(resolver);
    if let Some(s) = strict {
        b = b.enforce_baseline_latency(s);
    }
    b.run()
}

#[test]
fn scenario_baseline_p95_violated_advisory_default() {
    let result = build_baseline_and_run(
        "latency-scenario-4",
        Duration::from_millis(10),
        Duration::from_millis(500),
        None,
    );
    let record = result.verdict_record();
    let dim = record
        .latency()
        .expect("baseline latency → dimension present");
    assert!(
        dim.advisory_violations() > 0,
        "should record advisory violations"
    );
    assert_eq!(dim.strict_violations(), 0);
    record.assert_latency(); // no-op under advisory
    assert!(result.passed());
}

#[test]
fn scenario_baseline_p95_violated_strict_via_builder() {
    let result = build_baseline_and_run(
        "latency-scenario-5",
        Duration::from_millis(10),
        Duration::from_millis(500),
        Some(true),
    );
    let record = result.verdict_record();
    let dim = record.latency().unwrap();
    assert!(dim.strict_violations() > 0);
    assert!(!result.passed());
    // Validate mode propagated to each baseline-derived evaluation.
    for ev in dim.evaluations() {
        assert_eq!(ev.mode(), LatencyEnforcementMode::Strict);
    }
}

// Scenario 11 — feasibility gate on small baseline + high percentile.
#[test]
fn scenario_p99_with_small_baseline_is_infeasible() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    feotest::experiment::MeasureExperiment::builder()
        .service_contract_id("latency-scenario-11")
        .service_contract(|| SleepingContract { latency: Duration::from_millis(10) })
        .samples(30)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = feotest::spec::SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new(
        "latency-scenario-11",
        &inputs,
        fixed_latency_trial(Duration::from_millis(10)),
    )
    .approach(threshold_first(30, 0.80))
    .threshold_origin(feotest::model::ThresholdOrigin::Sla)
    .spec_resolver(resolver)
    .run();

    let record = result.verdict_record();
    let dim = record.latency().expect("baseline → dimension present");
    let p99 = dim
        .evaluations()
        .iter()
        .find(|e| e.percentile() == feotest::latency::Percentile::P99)
        .expect("p99 evaluation produced");
    assert_eq!(p99.status(), EvaluationStatus::Infeasible);
    assert!(
        record
            .warnings()
            .iter()
            .any(|w| w.code() == "LATENCY_INFEASIBLE")
    );
}

// Scenario 9 — MEASURE round-trip: vector persists, content fingerprint stable.
#[test]
fn measure_round_trip_preserves_latency_block() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let m = feotest::experiment::MeasureExperiment::builder()
        .service_contract_id("latency-scenario-9")
        .service_contract(|| SleepingContract { latency: Duration::from_millis(42) })
        .samples(50)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    let original = m
        .spec()
        .statistics
        .latency_distribution
        .as_ref()
        .expect("MEASURE populates latency block");
    assert_eq!(original.latencies_ms.len(), 50);
    assert!(original.latencies_ms.windows(2).all(|w| w[0] <= w[1]));
    assert!(
        (42..=200).contains(&original.mean_ms),
        "mean {} should be at/above the 42ms sleep floor",
        original.mean_ms
    );
    assert!(original.max_ms >= 42);

    // Reload via integrity-verifying path; fingerprint must match.
    let yaml = std::fs::read_to_string(m.spec_path().unwrap()).unwrap();
    let reloaded = feotest::spec::BaselineSpec::from_yaml(&yaml).expect("integrity ok");
    assert_eq!(
        reloaded.statistics.latency_distribution.as_ref(),
        Some(original)
    );
}
