//! Integration tests for the latency dimension.
//!
//! Covers the acceptance scenarios enumerated in
//! `plan/DES-LATENCY.md::Acceptance tests`.

use std::time::Duration;

use feotest::latency::{EvaluationStatus, LatencyCriterion, Percentile};
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::verdict::EnforcementMode;
use feotest::verdict::Verdict;

const fn threshold_first(samples: u32, pass_rate: f64) -> ThresholdApproach {
    ThresholdApproach::ThresholdFirst {
        samples,
        min_pass_rate: pass_rate,
    }
}

/// A single always-pass criterion for fixtures that exercise the latency path
/// rather than response judging.
fn trivial_criteria() -> feotest::criteria::Criteria<String> {
    feotest::criteria::Criteria::of([feotest::criteria::Criterion::meeting()
        .pass_rate(0.5)
        .name("response received")
        .satisfies("response received", |_: &String| Ok(()))
        .build()])
}

/// A contract that sleeps a fixed (small) duration on every invocation, so the
/// engine measures that latency from real invoke-elapsed time — there is no
/// synthetic-latency seam. It may optionally declare a p95 ceiling, and
/// percentiles asserted against the baseline.
struct SleepingContract {
    id: String,
    latency: Duration,
    p95_ceiling: Option<Duration>,
    against_baseline: &'static [Percentile],
}

impl SleepingContract {
    fn new(id: impl Into<String>, latency: Duration) -> Self {
        Self {
            id: id.into(),
            latency,
            p95_ceiling: None,
            against_baseline: &[],
        }
    }

    const fn p95(mut self, ceiling: Duration) -> Self {
        self.p95_ceiling = Some(ceiling);
        self
    }

    const fn against_baseline(mut self, percentiles: &'static [Percentile]) -> Self {
        self.against_baseline = percentiles;
        self
    }
}

impl feotest::service_contract::ServiceContract for SleepingContract {
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
        std::thread::sleep(self.latency);
        Ok(input.clone())
    }

    fn criteria(&self) -> feotest::criteria::Criteria<String> {
        trivial_criteria()
    }

    fn latency(&self) -> Option<LatencyCriterion> {
        if self.p95_ceiling.is_none() && self.against_baseline.is_empty() {
            return None;
        }
        let declared = self
            .p95_ceiling
            .map_or_else(LatencyCriterion::meeting, |c| {
                LatencyCriterion::meeting().at_most(Percentile::P95, c)
            });
        Some(
            self.against_baseline
                .iter()
                .fold(declared, |criterion, &p| criterion.against_baseline(p)),
        )
    }
}

// Scenario 1 — no latency config → dimension absent, assert_all ≡ assert_contract.
#[test]
fn scenario_no_latency_config_dimension_absent() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(SleepingContract::new(
        "latency-scenario-1",
        Duration::from_millis(1),
    ))
    .inputs(&inputs)
    .approach(threshold_first(30, 0.80))
    .run();

    assert!(result.verdict_record().latency().is_none());
    assert!(result.passed());
    result.verdict_record().assert_all();
}

// Scenario 2 — explicit p95 met → dimension present and enforced, pass.
// An explicit ceiling is a requirement decided by
// latency/compliance-exact-binomial: 59 successful latencies are the fewest
// from which any count can demonstrate a p95, so the scenarios bounding p95
// run 60 samples (fewer is refused under verification intent).
#[test]
fn scenario_explicit_p95_met() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(
        SleepingContract::new("latency-scenario-2", Duration::from_millis(10))
            .p95(Duration::from_millis(50)),
    )
    .inputs(&inputs)
    .approach(threshold_first(60, 0.80))
    .run();

    let record = result.verdict_record();
    let dim = record.latency().expect("latency dimension present");
    assert_eq!(dim.mode(), EnforcementMode::Enforced);
    assert_eq!(dim.verdict(), Some(Verdict::Pass));
    assert_eq!(dim.evaluations().len(), 1);
    assert_eq!(dim.evaluations()[0].status(), EvaluationStatus::Pass);
    assert!(result.passed());
    record.assert_all();
}

// Scenario 3 — explicit p95 violated → assert_latency panics, assert_contract ok.
#[test]
fn scenario_explicit_p95_violated_overall_fail() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(
        SleepingContract::new("latency-scenario-3", Duration::from_millis(30))
            .p95(Duration::from_millis(5)),
    )
    .inputs(&inputs)
    .approach(threshold_first(60, 0.80))
    .run();

    let record = result.verdict_record();
    assert_eq!(
        record.functional_assessment().composite(),
        Verdict::Pass,
        "functional still passes"
    );
    assert_eq!(
        record.verdict(),
        Some(Verdict::Fail),
        "the test verdict composes latency"
    );
    assert!(!result.passed(), "overall must fail due to latency");
    let dim = record.latency().unwrap();
    assert_eq!(dim.evaluations()[0].status(), EvaluationStatus::Fail);
    record.assert_contract(); // functional ok
}

#[test]
#[should_panic(expected = "latency contract failed")]
fn scenario_explicit_p95_violated_assert_latency_panics() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(
        SleepingContract::new("latency-scenario-3b", Duration::from_millis(30))
            .p95(Duration::from_millis(5)),
    )
    .inputs(&inputs)
    .approach(threshold_first(60, 0.80))
    .run();
    result.verdict_record().assert_latency();
}

// Scenario 4 — a p95 asserted against the baseline and violated is enforced
// by default, like an explicit ceiling: there is no default that depends on
// the threshold's source. The advisory switch is run-time only (`FEOTEST_ADVISORY`); its
// end-to-end behaviour is exercised in `tests/assertion_enforcement.rs`,
// which sets the variable in a child process so that these tests remain
// parallel-safe.
fn build_baseline_and_run(
    test_name: &str,
    baseline_latency: Duration,
    test_latency: Duration,
    against_baseline: &'static [Percentile],
) -> feotest::ptest::ProbabilisticTestResult {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    // Establish baseline with low-latency samples (the contract sleeps so the
    // engine measures a real latency).
    let baseline_id = test_name.to_owned();
    feotest::experiment::MeasureExperiment::builder()
        .service_contract_id(test_name)
        .service_contract(move || SleepingContract::new(baseline_id.clone(), baseline_latency))
        .samples(150)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = feotest::spec::SpecResolver::with_dir(dir.path());
    ProbabilisticTest::for_contract(
        SleepingContract::new(test_name, test_latency).against_baseline(against_baseline),
    )
    .inputs(&inputs)
    .approach(threshold_first(30, 0.80))
    .threshold_origin(feotest::model::ThresholdOrigin::Sla)
    .spec_resolver(resolver)
    .run()
}

#[test]
fn scenario_baseline_p95_violated_is_enforced_by_default() {
    let result = build_baseline_and_run(
        "latency-scenario-4",
        Duration::from_millis(3),
        Duration::from_millis(60),
        &[Percentile::P95],
    );
    let record = result.verdict_record();
    let dim = record
        .latency()
        .expect("baseline latency → dimension present");
    assert_eq!(dim.mode(), EnforcementMode::Enforced);
    let p95 = dim
        .evaluations()
        .iter()
        .find(|e| e.percentile() == Percentile::P95)
        .expect("p95 evaluation produced");
    assert_eq!(p95.status(), EvaluationStatus::Fail);
    assert_eq!(dim.verdict(), Some(Verdict::Fail));
    assert!(!result.passed());
    assert!(record.triggering().iter().any(|t| t.id() == "p95"));
}

#[test]
#[should_panic(expected = "latency contract failed")]
fn scenario_baseline_p95_violated_fails_assert_latency() {
    let result = build_baseline_and_run(
        "latency-scenario-5",
        Duration::from_millis(3),
        Duration::from_millis(60),
        &[Percentile::P95],
    );
    result.verdict_record().assert_latency();
}

// A baseline's latencies assert nothing the contract does not declare: with
// no percentile asserted against the baseline there is no latency dimension.
#[test]
fn scenario_undeclared_percentiles_are_not_asserted() {
    let result = build_baseline_and_run(
        "latency-scenario-4b",
        Duration::from_millis(3),
        Duration::from_millis(60),
        &[],
    );
    assert!(result.verdict_record().latency().is_none());
    assert!(result.passed());
}

// Scenario 11 — a small baseline and a high percentile: no baseline rank
// achieves alpha at this test size, so the p99 is saturated — warned before
// the run, decided INCONCLUSIVE after it.
#[test]
fn scenario_p99_with_small_baseline_is_saturated() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    feotest::experiment::MeasureExperiment::builder()
        .service_contract_id("latency-scenario-11")
        .service_contract(|| {
            SleepingContract::new("latency-scenario-11", Duration::from_millis(10))
        })
        .samples(30)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = feotest::spec::SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTest::for_contract(
        SleepingContract::new("latency-scenario-11", Duration::from_millis(10))
            .against_baseline(&[Percentile::P99]),
    )
    .inputs(&inputs)
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
    assert_eq!(p99.status(), EvaluationStatus::Saturated);
    assert_eq!(p99.threshold(), None);
    assert_eq!(record.verdict(), Some(Verdict::Inconclusive));
    assert!(
        record
            .warnings()
            .iter()
            .any(|w| w.code() == "LATENCY_SATURATION_EXPECTED")
    );
}

// Scenario 9 — MEASURE round-trip: vector persists, content fingerprint stable.
#[test]
fn measure_round_trip_preserves_latency_block() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    let m = feotest::experiment::MeasureExperiment::builder()
        .service_contract_id("latency-scenario-9")
        .service_contract(|| SleepingContract::new("latency-scenario-9", Duration::from_millis(42)))
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
