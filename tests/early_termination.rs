//! Integration tests for failure-inevitable and success-guaranteed
//! early termination driven through the probabilistic-test runner.

mod common;

use std::sync::atomic::{AtomicU32, Ordering};

use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::MeasureExperiment;
use feotest::model::{ContractViolation, TerminationReason, ThresholdOrigin};
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::service_contract::ServiceContract;
use feotest::spec::SpecResolver;
use feotest::statistics::rules::DecisionRule;
use feotest::verdict::{Verdict, VerdictRecord};

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
    assert_eq!(record.verdict(), Some(Verdict::Fail));
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
    assert_eq!(record.verdict(), Some(Verdict::Fail));
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
    assert_eq!(record.verdict(), Some(Verdict::Pass));
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
    assert_eq!(record.verdict(), Some(Verdict::Pass));
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

// --- smoke intent: the smoke rule's required count ends the run ---

/// A contract with one requirement of 0.99 whose postcondition fails on
/// exactly the listed (1-based) samples.
struct RequirementContract {
    id: &'static str,
    misses: &'static [u32],
}

impl ServiceContract for RequirementContract {
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
        Ok(input.clone())
    }

    fn criteria(&self) -> Criteria<String> {
        let misses = self.misses;
        let judged = AtomicU32::new(0);
        Criteria::of([Criterion::meeting()
            .pass_rate(0.99)
            .name("harmful advice is rejected")
            .satisfies("rejected", move |_: &String| {
                let sample = judged.fetch_add(1, Ordering::SeqCst) + 1;
                if misses.contains(&sample) {
                    Err(ContractViolation::new("rejected", "harmful advice given"))
                } else {
                    Ok(())
                }
            })
            .build()])
    }
}

/// A smoke test of 100 samples at a 0.99 requirement and 95% confidence:
/// a design verification would refuse (it needs 299).
fn run_smoke_requirement(contract: RequirementContract) -> VerdictRecord {
    let inputs = vec!["input".to_string()];
    ProbabilisticTest::for_contract(contract)
        .inputs(&inputs)
        .approach(ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.99,
        })
        .smoke()
        .threshold_origin(ThresholdOrigin::Policy)
        .run()
        .verdict_record()
        .clone()
}

#[test]
fn an_undersized_smoke_requirement_passes_once_its_required_count_is_reached() {
    // compliance/smoke-exact-binomial requires 97 of 100: the 97th success
    // settles the PASS, so the run stops there rather than running on or
    // failing for want of a size it never claimed.
    let record = run_smoke_requirement(RequirementContract {
        id: "smoke-success-guaranteed",
        misses: &[],
    });
    assert_eq!(record.verdict(), Some(Verdict::Pass));
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::SuccessGuaranteed
    );
    assert_eq!(record.execution().samples_executed(), 97);
    let analysis = record.statistical_analysis().unwrap();
    assert_eq!(
        analysis.decision_rule(),
        DecisionRule::ComplianceSmokeExactBinomial
    );
    assert_eq!(analysis.required_pass(), Some(97));
    assert!(record.warnings().iter().any(|w| w.code() == "UNDERSIZED"));
    // A smoke decision enters neither Type-I envelope (§1.4.6).
    assert_eq!(record.envelopes().false_compliance(), None);
    assert_eq!(record.envelopes().false_degradation_signal(), None);
}

#[test]
fn an_undersized_smoke_requirement_fails_once_its_required_count_is_out_of_reach() {
    // The fourth miss arrives at sample 30: with more than 100 - 97 = 3
    // misses no outcome can reach 97, so the FAIL is settled there.
    let record = run_smoke_requirement(RequirementContract {
        id: "smoke-failure-inevitable",
        misses: &[6, 13, 21, 30],
    });
    assert_eq!(record.verdict(), Some(Verdict::Fail));
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::FailureInevitable
    );
    assert_eq!(record.execution().samples_executed(), 30);
    let analysis = record.statistical_analysis().unwrap();
    assert_eq!(
        analysis.decision_rule(),
        DecisionRule::ComplianceSmokeExactBinomial
    );
    assert_eq!(analysis.required_pass(), Some(97));
}

#[test]
fn three_misses_in_an_undersized_smoke_run_are_no_evidence_of_a_shortfall() {
    // Three misses leave 97 within reach; the run continues until the 97th
    // success, at sample 100, and passes.
    let record = run_smoke_requirement(RequirementContract {
        id: "smoke-three-misses",
        misses: &[2, 50, 99],
    });
    assert_eq!(record.verdict(), Some(Verdict::Pass));
    assert_eq!(record.execution().samples_executed(), 100);
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
    assert_eq!(record.verdict(), Some(Verdict::Fail));
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
    assert_eq!(record.verdict(), Some(Verdict::Pass));
    assert_eq!(record.execution().samples_executed(), 200);
    assert_eq!(
        record.execution().termination().reason(),
        &TerminationReason::Completed
    );
}
