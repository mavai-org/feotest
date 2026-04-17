//! Integration tests for the latency dimension (LT01–LT06).
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
fn fixed_latency_trial(latency: Duration) -> impl FnMut(&str) -> TrialOutcome {
    move |_input: &str| TrialOutcome::success(latency)
}

fn threshold_first(samples: u32, pass_rate: f64) -> ThresholdApproach {
    ThresholdApproach::ThresholdFirst {
        samples,
        min_pass_rate: pass_rate,
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
    struct Uc {
        id: &'static str,
    }
    impl feotest::usecase::UseCase for Uc {
        fn id(&self) -> &str {
            self.id
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let uc_id: &'static str = Box::leak(test_name.to_string().into_boxed_str());
    let uc = Uc { id: uc_id };
    let inputs = vec!["input".to_string()];

    // Establish baseline with low-latency samples.
    feotest::experiment::MeasureExperiment::new(
        &uc,
        150,
        &inputs,
        fixed_latency_trial(baseline_latency),
    )
    .with_spec_resolver(feotest::spec::SpecResolver::with_dir(dir.path()))
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
    struct Uc;
    impl feotest::usecase::UseCase for Uc {
        fn id(&self) -> &str {
            "latency-scenario-11"
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    feotest::experiment::MeasureExperiment::new(
        &Uc,
        30,
        &inputs,
        fixed_latency_trial(Duration::from_millis(10)),
    )
    .with_spec_resolver(feotest::spec::SpecResolver::with_dir(dir.path()))
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
    struct Uc;
    impl feotest::usecase::UseCase for Uc {
        fn id(&self) -> &str {
            "latency-scenario-9"
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let m = feotest::experiment::MeasureExperiment::new(
        &Uc,
        50,
        &inputs,
        fixed_latency_trial(Duration::from_millis(42)),
    )
    .with_spec_resolver(feotest::spec::SpecResolver::with_dir(dir.path()))
    .run();

    let original = m
        .spec()
        .statistics
        .latency_distribution
        .as_ref()
        .expect("MEASURE populates latency block");
    assert_eq!(original.latencies_ms.len(), 50);
    assert!(original.latencies_ms.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(original.mean_ms, 42);
    assert_eq!(original.max_ms, 42);

    // Reload via integrity-verifying path; fingerprint must match.
    let yaml = std::fs::read_to_string(m.spec_path().unwrap()).unwrap();
    let reloaded = feotest::spec::BaselineSpec::from_yaml(&yaml).expect("integrity ok");
    assert_eq!(
        reloaded.statistics.latency_distribution.as_ref(),
        Some(original)
    );
}
