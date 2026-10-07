//! The run-time enforcement switch, end to end.
//!
//! `FEOTEST_ADVISORY` is read from the process environment when a test runs.
//! Setting a variable in a running test process would race every other test
//! reading it (and `std::env::set_var` is `unsafe`, which the crate forbids),
//! so each scenario runs in a child process: the parent test re-executes
//! this test binary with the variable set and only the scenario's ignored
//! child test selected, and asserts that the child passed. A child run
//! without its parent skips, so `--include-ignored` runs stay green.

use std::process::Command;
use std::time::Duration;

use feotest::criteria::{Criteria, Criterion};
use feotest::latency::{LatencyCriterion, Percentile};
use feotest::model::{ContractViolation, TerminationReason, ThresholdOrigin};
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::reporting::VerdictXmlWriter;
use feotest::service_contract::ServiceContract;
use feotest::spec::SpecResolver;
use feotest::verdict::enforcement::ENV_VAR;
use feotest::verdict::{EnforcementMode, Verdict};

/// Marks a child process started by its parent test.
const CHILD_MARKER: &str = "FEOTEST_ENFORCEMENT_SCENARIO";

/// Re-executes this test binary running only the ignored child test
/// `child`, with `FEOTEST_ADVISORY` set to `advisory`, and asserts that it
/// ran and passed.
fn run_child(child: &str, advisory: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", child, "--ignored", "--nocapture"])
        .env(ENV_VAR, advisory)
        .env(CHILD_MARKER, child)
        .output()
        .expect("re-execute the test binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "child {child} with {ENV_VAR}={advisory} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Whether this process is the child its parent started for `child`; a
/// child run any other way skips.
fn is_child(child: &str) -> bool {
    let started = std::env::var(CHILD_MARKER).is_ok_and(|marker| marker == child);
    if !started {
        eprintln!("skipping: {child} runs only under its parent test");
    }
    started
}

/// A contract whose single criterion requires `pass_rate` and holds on every
/// `pass_every`-th sample (1: every sample), and which declares an explicit
/// latency ceiling at `percentile`. Each invocation sleeps `sleep`.
struct ScenarioContract {
    id: &'static str,
    pass_rate: Option<f64>,
    pass_every: u32,
    sleep: Duration,
    ceiling: Option<(Percentile, Duration)>,
}

impl ScenarioContract {
    const fn new(id: &'static str) -> Self {
        Self {
            id,
            pass_rate: Some(0.5),
            pass_every: 1,
            sleep: Duration::ZERO,
            ceiling: None,
        }
    }
}

impl ServiceContract for ScenarioContract {
    type Input = String;
    type Output = String;

    fn id(&self) -> &str {
        self.id
    }

    fn invoke(
        &self,
        input: &String,
        _cost: &mut feotest::controls::Cost,
    ) -> Result<String, feotest::model::Defect> {
        std::thread::sleep(self.sleep);
        Ok(input.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        let counter = std::sync::atomic::AtomicU32::new(0);
        let every = self.pass_every;
        let check = move |_: &String| {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n % every == 0 {
                Ok(())
            } else {
                Err(ContractViolation::new("scripted", "scripted failure"))
            }
        };
        let criterion = match self.pass_rate {
            Some(rate) => Criterion::meeting()
                .pass_rate(rate)
                .name("scripted")
                .satisfies("scripted", check)
                .build(),
            None => Criterion::empirical()
                .pass_rate()
                .name("scripted")
                .satisfies("scripted", check)
                .build(),
        };
        Criteria::of([criterion])
    }

    fn latency(&self) -> Option<LatencyCriterion> {
        self.ceiling
            .map(|(percentile, ceiling)| LatencyCriterion::meeting().at_most(percentile, ceiling))
    }
}

/// A run of `samples` with an early-termination floor of `floor`.
const fn threshold_first(samples: u32, floor: f64) -> ThresholdApproach {
    ThresholdApproach::ThresholdFirst {
        samples,
        min_pass_rate: floor,
    }
}

/// Every invocation sleeps 3 ms against a p95 ceiling of 1 ms: the explicit
/// requirement FAILs by its rule.
const fn slow_service(id: &'static str) -> ScenarioContract {
    ScenarioContract {
        sleep: Duration::from_millis(3),
        ceiling: Some((Percentile::P95, Duration::from_millis(1))),
        ..ScenarioContract::new(id)
    }
}

/// Half the samples fail a 0.9 requirement, under a generous p50 ceiling
/// the passing samples all meet.
const fn half_failing_service(id: &'static str) -> ScenarioContract {
    ScenarioContract {
        pass_rate: Some(0.9),
        pass_every: 2,
        ceiling: Some((Percentile::P50, Duration::from_secs(5))),
        ..ScenarioContract::new(id)
    }
}

// --- Enforced by default ---------------------------------------------------

#[test]
fn unset_every_assertion_is_enforced() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(slow_service("enforced-latency"))
        .inputs(&inputs)
        .approach(threshold_first(60, 0.0))
        .run();
    let record = result.verdict_record();
    let latency = record.latency().unwrap();
    assert_eq!(latency.mode(), EnforcementMode::Enforced);
    assert_eq!(latency.verdict(), Some(Verdict::Fail));
    assert_eq!(record.verdict(), Some(Verdict::Fail));
    assert_eq!(
        record.functional_assessment().mode(),
        EnforcementMode::Enforced
    );
}

#[test]
fn unset_an_inevitable_functional_failure_ends_the_run_early() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(half_failing_service("enforced-early"))
        .inputs(&inputs)
        .approach(threshold_first(100, 0.9))
        .run();
    let execution = result.verdict_record().execution();
    assert_eq!(
        *execution.termination().reason(),
        TerminationReason::FailureInevitable
    );
    assert!(execution.samples_executed() < 100);
}

// --- latency ---------------------------------------------------------------

#[test]
fn latency_advisory_reports_a_latency_fail_without_failing_the_test() {
    run_child("child_latency_advisory", "latency");
}

#[test]
#[ignore = "run by its parent test with FEOTEST_ADVISORY set"]
fn child_latency_advisory() {
    if !is_child("child_latency_advisory") {
        return;
    }
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(slow_service("advisory-latency"))
        .inputs(&inputs)
        .approach(threshold_first(60, 0.0))
        .run();
    let record = result.verdict_record();
    let latency = record.latency().unwrap();
    assert_eq!(latency.mode(), EnforcementMode::Advisory);
    assert_eq!(latency.verdict(), Some(Verdict::Fail));
    assert_eq!(record.verdict(), Some(Verdict::Pass));
    assert_eq!(record.triggering(), []);
    record.assert_latency();
    record.assert_all();
    let xml = VerdictXmlWriter::write_record(record, Some("2026-10-07T12:00:00Z"));
    assert!(xml.contains("verdict=\"FAIL\" mode=\"advisory\""));
    assert!(xml.contains("<composite value=\"PASS\" mode=\"enforced\"/>"));
    assert!(xml.contains("<verdict value=\"PASS\""));
    assert_validates(&xml);
}

/// Validates a written record against the vendored verdict schema; skips
/// when `xmllint` is not installed.
fn assert_validates(xml: &str) {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), xml).unwrap();
    let schema = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/conformance/interchange/verdict-1.8.xsd"
    );
    match Command::new("xmllint")
        .args(["--noout", "--schema", schema])
        .arg(file.path())
        .output()
    {
        Ok(output) => assert!(
            output.status.success(),
            "the record violates the published schema:\n{}\n{xml}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping schema validation: xmllint not installed");
        }
        Err(error) => panic!("failed to run xmllint: {error}"),
    }
}

#[test]
fn latency_advisory_still_refuses_an_undecidable_latency_design() {
    run_child("child_latency_advisory_refusal", "latency");
}

#[test]
#[ignore = "run by its parent test with FEOTEST_ADVISORY set"]
fn child_latency_advisory_refusal() {
    if !is_child("child_latency_advisory_refusal") {
        return;
    }
    // No count of 30 successful latencies can demonstrate a p95.
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(slow_service("advisory-latency-refused"))
        .inputs(&inputs)
        .approach(threshold_first(30, 0.0))
        .run();
    let record = result.verdict_record();
    assert!(record.is_refused());
    assert_eq!(
        record.configuration_errors()[0].code(),
        "COMPLIANCE_INFEASIBLE"
    );
    let refused = std::panic::catch_unwind(|| record.assert_all());
    assert!(
        refused.is_err(),
        "a refusal fails the test whatever the switch"
    );
}

// --- functional ------------------------------------------------------------

#[test]
fn functional_advisory_runs_every_latency_sample() {
    run_child("child_functional_advisory", "functional");
}

#[test]
#[ignore = "run by its parent test with FEOTEST_ADVISORY set"]
fn child_functional_advisory() {
    if !is_child("child_functional_advisory") {
        return;
    }
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(half_failing_service("advisory-functional"))
        .inputs(&inputs)
        .approach(threshold_first(100, 0.9))
        .run();
    let record = result.verdict_record();
    // The inevitable functional failure does not cut the run short.
    assert_eq!(record.execution().samples_executed(), 100);
    assert_eq!(
        *record.execution().termination().reason(),
        TerminationReason::Completed
    );
    let functional = record.functional_assessment();
    assert_eq!(functional.mode(), EnforcementMode::Advisory);
    assert_eq!(functional.composite(), Verdict::Fail);
    assert_eq!(record.latency().unwrap().verdict(), Some(Verdict::Pass));
    assert_eq!(record.verdict(), Some(Verdict::Pass));
    record.assert_contract();
    record.assert_all();
}

#[test]
fn functional_advisory_runs_without_a_baseline() {
    run_child("child_functional_advisory_no_baseline", "functional");
}

#[test]
#[ignore = "run by its parent test with FEOTEST_ADVISORY set"]
fn child_functional_advisory_no_baseline() {
    if !is_child("child_functional_advisory_no_baseline") {
        return;
    }
    let empty = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(ScenarioContract {
        pass_rate: None,
        ..ScenarioContract::new("advisory-no-baseline")
    })
    .inputs(&inputs)
    .approach(ThresholdApproach::SampleSizeFirst {
        samples: 50,
        confidence: 0.95,
    })
    .threshold_origin(ThresholdOrigin::Empirical)
    .spec_resolver(SpecResolver::with_dir(empty.path()))
    .run();
    let record = result.verdict_record();
    assert_eq!(record.execution().samples_executed(), 50);
    let row = &record.functional_assessment().criteria()[0];
    assert_eq!(row.verdict(), Verdict::Inconclusive);
    assert!(row.statistical_analysis().is_none());
    assert_eq!(record.verdict(), Some(Verdict::Pass));
    record.assert_all();
}

// --- both ------------------------------------------------------------------

#[test]
fn both_advisory_decides_and_reports_both_dimensions() {
    run_child("child_both_advisory", "Latency, FUNCTIONAL");
}

#[test]
#[ignore = "run by its parent test with FEOTEST_ADVISORY set"]
fn child_both_advisory() {
    if !is_child("child_both_advisory") {
        return;
    }
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(ScenarioContract {
        pass_rate: Some(0.9),
        pass_every: 2,
        ..slow_service("advisory-both")
    })
    .inputs(&inputs)
    .approach(threshold_first(200, 0.9))
    .run();
    let record = result.verdict_record();
    assert_eq!(record.functional_assessment().composite(), Verdict::Fail);
    assert_eq!(record.latency().unwrap().verdict(), Some(Verdict::Fail));
    assert_eq!(record.verdict(), Some(Verdict::Pass));
    assert!(record.envelopes().false_compliance().is_none());
    record.assert_contract();
    record.assert_latency();
    record.assert_all();
}

// --- misconfiguration ------------------------------------------------------

#[test]
fn an_unknown_setting_is_a_configuration_error() {
    run_child("child_unknown_setting", "strict");
}

#[test]
#[ignore = "run by its parent test with FEOTEST_ADVISORY set"]
fn child_unknown_setting() {
    if !is_child("child_unknown_setting") {
        return;
    }
    let inputs = vec!["input".to_string()];
    let refused = std::panic::catch_unwind(|| {
        ProbabilisticTest::for_contract(ScenarioContract::new("advisory-unknown"))
            .inputs(&inputs)
            .approach(threshold_first(10, 0.0))
            .run()
    });
    let payload = refused.expect_err("an unknown setting stops the run");
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(
        message.contains("FEOTEST_ADVISORY must name functional, latency or both"),
        "{message}"
    );
}
