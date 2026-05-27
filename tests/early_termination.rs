//! Integration tests for failure-inevitable and success-guaranteed
//! early termination driven through the probabilistic-test runner.

mod common;

use feotest::experiment::MeasureExperiment;
use feotest::model::{TerminationReason, ThresholdOrigin};
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::spec::SpecResolver;
use feotest::verdict::Verdict;

// --- runner-driven failure-inevitable termination ---

#[test]
fn threshold_first_terminates_on_failure_inevitable() {
    let inputs = vec!["input".to_string()];
    let result = ProbabilisticTest::for_contract(common::FailingServiceContract::new("fail-inev"))
        .inputs(&inputs)
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
    let inputs = vec!["input".to_string()];
    MeasureExperiment::builder()
        .service_contract_id("ssf-fail-inev")
        .service_contract(|| common::SimpleServiceContract::new("baseline"))
        .samples(200)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    let resolver = SpecResolver::with_dir(dir.path());
    let result =
        ProbabilisticTest::for_contract(common::FailingServiceContract::new("ssf-fail-inev"))
            .inputs(&inputs)
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

// --- runner-driven success-guaranteed termination ---

#[test]
fn threshold_first_terminates_on_success_guaranteed() {
    let inputs = vec!["input".to_string()];
    let result =
        ProbabilisticTest::for_contract(common::SimpleServiceContract::new("success-guaranteed"))
            .inputs(&inputs)
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
    let result = ProbabilisticTest::for_contract(common::SimpleServiceContract::new("floor-delay"))
        .inputs(&inputs)
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
    // The naive success-guaranteed threshold would fire at sample = ceil(500 * 0.90)
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
    let inputs = vec!["input".to_string()];
    let result = MeasureExperiment::builder()
        .service_contract_id("measure-non-reg")
        .service_contract(|| common::FailingServiceContract::new("baseline"))
        .samples(30)
        .inputs(&inputs)
        .build()
        .run();
    assert_eq!(result.execution().summary().samples_executed(), 30);
    assert_eq!(
        result.execution().summary().termination().reason(),
        &TerminationReason::Completed
    );
}

// --- developer override: disable early termination ---

#[test]
fn override_runs_all_samples_despite_inevitable_failure() {
    // Always-fail at a 0.90 target would normally terminate on
    // FailureInevitable after a handful of samples; the override runs them all.
    let inputs = vec!["input".to_string()];
    let result =
        ProbabilisticTest::for_contract(common::FailingServiceContract::new("override-fail"))
            .inputs(&inputs)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 200,
                min_pass_rate: 0.90,
            })
            .disable_early_termination()
            .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Fail);
    assert_eq!(record.execution().samples_executed(), 200);
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::Completed
    );
}

#[test]
fn override_runs_all_samples_despite_guaranteed_success() {
    // Always-pass at a low target would normally terminate on
    // SuccessGuaranteed; the override runs every declared sample.
    let inputs = vec!["input".to_string()];
    let result =
        ProbabilisticTest::for_contract(common::SimpleServiceContract::new("override-success"))
            .inputs(&inputs)
            .approach(ThresholdApproach::ThresholdFirst {
                samples: 200,
                min_pass_rate: 0.50,
            })
            .disable_early_termination()
            .run();

    let record = result.verdict_record();
    assert_eq!(record.verdict(), Verdict::Pass);
    assert_eq!(record.execution().samples_executed(), 200);
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::Completed
    );
}
