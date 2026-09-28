//! Risk-driven sizing through the contract-driven probabilistic test.
//!
//! Drives a scripted contract end to end: a measure experiment establishes a
//! baseline at a known count, the test declares a risk appetite (the design
//! alternative rate, confidence, target power), and the runner computes the
//! sample count resolved sizing against the observed baseline prices for
//! that promise (Statistical Companion §5.4.1).

use std::sync::atomic::{AtomicU32, Ordering};

use feotest::criteria::{Criteria, Criterion};
use feotest::experiment::MeasureExperiment;
use feotest::model::{ContractViolation, ThresholdOrigin};
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::service_contract::ServiceContract;
use feotest::spec::SpecResolver;
use feotest::statistics::risk_driven_sizing;

/// One criterion's script: its name and how many judged samples pass before
/// every subsequent one fails.
struct CriterionScript {
    name: &'static str,
    passing: u32,
}

/// A contract whose baseline-derived criteria each pass on exactly the first
/// `passing` judged samples — a deterministic script for reproducing exact
/// per-criterion baseline rates through the production measurement loop.
struct ScriptedContract {
    id: String,
    scripts: Vec<CriterionScript>,
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
        fn scripted(script: &CriterionScript) -> Criterion<String> {
            let passing = script.passing;
            let judged = AtomicU32::new(0);
            Criterion::empirical()
                .pass_rate()
                .name(script.name)
                .satisfies(script.name, move |_: &String| {
                    if judged.fetch_add(1, Ordering::SeqCst) < passing {
                        Ok(())
                    } else {
                        Err(ContractViolation::new("scripted", "scripted failure"))
                    }
                })
                .build()
        }
        match self.scripts.as_slice() {
            [only] => Criteria::of([scripted(only)]),
            [first, second] => Criteria::of([scripted(first), scripted(second)]),
            other => panic!("unsupported script count: {}", other.len()),
        }
    }
}

/// Establishes a baseline of `trials` samples in a fresh temp directory
/// (returned to keep it alive), with each scripted criterion passing exactly
/// its scripted count.
fn establish_scripted_baseline(
    service_contract_id: &str,
    scripts: &'static [(&'static str, u32)],
    trials: u32,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let id = service_contract_id.to_owned();

    MeasureExperiment::builder()
        .service_contract_id(service_contract_id)
        .service_contract(move || ScriptedContract {
            id: id.clone(),
            scripts: scripts
                .iter()
                .map(|&(name, passing)| CriterionScript { name, passing })
                .collect(),
        })
        .samples(trials)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    dir
}

/// Runs a risk-driven test against the scripted baseline with every test
/// sample passing.
fn run_risk_driven(
    service_contract_id: &str,
    baseline_dir: &std::path::Path,
    criterion_names: &'static [&'static str],
    design_alternative_rate: f64,
) -> feotest::ptest::ProbabilisticTestResult {
    let inputs = vec!["input".to_string()];
    ProbabilisticTest::for_contract(ScriptedContract {
        id: service_contract_id.to_owned(),
        scripts: criterion_names
            .iter()
            .map(|&name| CriterionScript {
                name,
                passing: u32::MAX,
            })
            .collect(),
    })
    .inputs(&inputs)
    .approach(ThresholdApproach::RiskDriven {
        design_alternative_rate,
        confidence: 0.95,
        target_power: 0.80,
    })
    .threshold_origin(ThresholdOrigin::Empirical)
    .spec_resolver(SpecResolver::with_dir(baseline_dir))
    .run()
}

#[test]
fn computed_sample_count_matches_the_oracle_for_the_walkthrough_baseline() {
    // Oracle case `resolved_walkthrough_1920_of_2000_at_093`: a baseline of
    // 1920 of 2000, a design alternative rate of 0.93 at 95% confidence and
    // 80% power resolves to 460 samples.
    let id = "risk-driven-walkthrough-baseline";
    let baseline_dir = establish_scripted_baseline(id, &[("accuracy", 1920)], 2000);

    let result = run_risk_driven(id, baseline_dir.path(), &["accuracy"], 0.93);

    let execution = result.verdict_record().execution();
    assert_eq!(execution.samples_planned(), 460);
    assert!(result.passed());
}

#[test]
fn governing_sample_count_is_the_weakest_criterions_requirement() {
    // Two baseline-derived criteria at different rates: the lower-rate
    // criterion sits closer to the design alternative and demands more
    // samples, so its requirement governs the whole run.
    let id = "risk-driven-governing";
    let baseline_dir = establish_scripted_baseline(
        id,
        &[("format valid", 1920), ("content faithful", 1900)],
        2000,
    );

    let result = run_risk_driven(
        id,
        baseline_dir.path(),
        &["format valid", "content faithful"],
        0.93,
    );

    let required = |successes| {
        risk_driven_sizing::resolved_sizing(successes, 2000, 0.93, 0.05, 0.80)
            .unwrap()
            .required_samples()
    };
    let governing = required(1900);
    let weaker = required(1920);
    assert!(
        governing > weaker,
        "the lower rate must demand more samples"
    );
    assert_eq!(
        result.verdict_record().execution().samples_planned(),
        governing
    );
}

#[test]
#[should_panic(
    expected = "sizing is undefined for criterion 'accuracy' (ALTERNATIVE_NOT_BELOW_BASELINE)"
)]
fn a_design_alternative_above_the_baseline_panics_naming_the_criterion() {
    // A design alternative rate of 0.97 against a baseline measured at 0.96
    // asks the test to detect a degradation the baseline already exceeds.
    let id = "risk-driven-over-reach";
    let baseline_dir = establish_scripted_baseline(id, &[("accuracy", 96)], 100);

    run_risk_driven(id, baseline_dir.path(), &["accuracy"], 0.97);
}

#[test]
#[should_panic(expected = "sizing is undefined for criterion 'accuracy' (BASELINE_TOO_SMALL)")]
fn a_baseline_too_small_for_the_design_panics_naming_the_criterion() {
    // Oracle case `resolved_baseline_too_small_288_of_300_at_093`.
    let id = "risk-driven-too-small";
    let baseline_dir = establish_scripted_baseline(id, &[("accuracy", 288)], 300);

    run_risk_driven(id, baseline_dir.path(), &["accuracy"], 0.93);
}
