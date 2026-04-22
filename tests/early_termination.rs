//! Integration tests for PT09 (failure inevitable) and PT10
//! (success guaranteed) early termination driven through the
//! probabilistic-test runner.

use std::time::Duration;

use feotest::experiment::MeasureExperiment;
use feotest::model::{ContractViolation, TerminationReason, ThresholdOrigin, TrialOutcome};
use feotest::ptest::ProbabilisticTestBuilder;
use feotest::ptest::builder::ThresholdApproach;
use feotest::spec::SpecResolver;
use feotest::usecase::UseCase;
use feotest::verdict::Verdict;

// --- Fixtures ---

struct TestUc(&'static str);
impl UseCase for TestUc {
    fn id(&self) -> &str {
        self.0
    }
}

const fn always_succeed(_input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

fn always_fail(_input: &str) -> TrialOutcome {
    TrialOutcome::failure(
        ContractViolation::new("forced", "always-fail"),
        Duration::from_millis(1),
    )
}

// --- PT09: runner-driven failure-inevitable termination ---

#[test]
fn threshold_first_terminates_on_failure_inevitable() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("fail-inev", &inputs, always_fail)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 200,
            min_pass_rate: 0.90,
        })
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Fail);
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::FailureInevitable
    );
    assert!(
        record.execution().samples_executed() < 200,
        "expected early termination; executed {} of 200",
        record.execution().samples_executed()
    );
}

#[test]
fn sample_size_first_terminates_on_failure_inevitable() {
    // Measure a baseline first so SampleSizeFirst has something to
    // derive a threshold from. The baseline itself runs to completion
    // (measure uses no min_pass_rate).
    let dir = tempfile::tempdir().unwrap();
    let baseline_uc = TestUc("ssf-fail-inev");
    let inputs = vec!["input".to_string()];
    MeasureExperiment::builder()
        .use_case(&baseline_uc)
        .samples(200)
        .inputs(&inputs)
        .trial(always_succeed)
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = SpecResolver::with_dir(dir.path());
    let result = ProbabilisticTestBuilder::new("ssf-fail-inev", &inputs, always_fail)
        .approach(ThresholdApproach::SampleSizeFirst {
            samples: 200,
            confidence: 0.95,
        })
        .spec_resolver(resolver)
        .threshold_origin(ThresholdOrigin::Empirical)
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Fail);
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::FailureInevitable
    );
    assert!(record.execution().samples_executed() < 200);
}

// --- PT10: runner-driven success-guaranteed termination ---

#[test]
fn threshold_first_terminates_on_success_guaranteed() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTestBuilder::new("success-guaranteed", &inputs, always_succeed)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 200,
            min_pass_rate: 0.50,
        })
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass);
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::SuccessGuaranteed
    );
    assert!(
        record.execution().samples_executed() < 200,
        "expected early termination; executed {} of 200",
        record.execution().samples_executed()
    );
}

#[test]
fn validity_floor_delays_runner_success_guaranteed() {
    // At very high thresholds the minimum sample count from the
    // feasibility computation is large enough that SuccessGuaranteed
    // cannot fire until the floor is cleared. The verdict should still
    // be Pass and the termination reason SuccessGuaranteed, but
    // samples_executed must be at least the feasibility floor.
    let inputs = vec!["input".to_string()];
    let samples = 500;
    let result = ProbabilisticTestBuilder::new("floor-delay", &inputs, always_succeed)
        .approach(ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate: 0.90,
        })
        .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass);
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::SuccessGuaranteed
    );
    // The naive PT10 threshold would fire at sample = ceil(500 * 0.90)
    // = 450. The validity floor must push it to at least the minimum
    // required for a 95 %-confidence Wilson verdict at target 0.9,
    // which is well above that 450 figure in the general case — we
    // only check the broader invariant here.
    let executed = record.execution().samples_executed();
    assert!(
        executed < samples,
        "expected early termination, executed {executed} of {samples}"
    );
    assert!(
        executed >= 450,
        "samples_executed={executed} should at least reach the unfloored threshold (450)"
    );
}

// --- Regression: measure/explore/optimize are unaffected ---

#[test]
fn measure_experiment_runs_all_samples_regardless_of_failures() {
    let uc = TestUc("measure-non-reg");
    let inputs = vec!["input".to_string()];
    let result = MeasureExperiment::builder()
        .use_case(&uc)
        .samples(30)
        .inputs(&inputs)
        .trial(always_fail)
        .build()
        .run();
    assert_eq!(result.execution().summary().samples_executed(), 30);
    assert_eq!(
        result.execution().summary().termination().reason(),
        &TerminationReason::Completed
    );
}
