//! Shared helpers for integration tests.
//!
//! Each integration-test binary compiles this module independently and uses a
//! different subset of the helpers, so unused items are expected per binary.
#![allow(dead_code)]

use std::path::Path;
use std::time::Duration;

use feotest::model::TrialOutcome;
use feotest::ptest::ProbabilisticTest;
use feotest::ptest::builder::ThresholdApproach;
use feotest::spec::SpecResolver;
use feotest::service_contract::ServiceContract;

// ---------------------------------------------------------------------------
// Trial closures
// ---------------------------------------------------------------------------

/// A trial that always succeeds with the given latency.
pub fn fixed_latency_trial(latency: Duration) -> impl FnMut(&str) -> TrialOutcome {
    move |_| TrialOutcome::success(latency)
}

/// A trial that always succeeds with 1ms latency.
pub fn always_succeeds(_input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

/// A trial that fails a fixed fraction of the time (deterministic by input index).
pub fn failing_trial(fail_rate: f64) -> impl FnMut(&str) -> TrialOutcome {
    let mut count = 0u64;
    move |_| {
        count += 1;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "test-only: fail_rate in [0, 1] so value <= 100"
        )]
        let threshold = (fail_rate * 100.0) as u64;
        if count % 100 < threshold {
            TrialOutcome::failure(
                feotest::model::ContractViolation::new("check", "forced"),
                Duration::from_millis(1),
            )
        } else {
            TrialOutcome::success(Duration::from_millis(1))
        }
    }
}

// ---------------------------------------------------------------------------
// Service contract helpers
// ---------------------------------------------------------------------------

/// A simple service contract with no covariates.
pub struct SimpleServiceContract {
    id: String,
}

impl SimpleServiceContract {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl ServiceContract for SimpleServiceContract {
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

    fn criteria(&self) -> feotest::criteria::Criteria<String> {
        feotest::criteria::Criteria::of([feotest::criteria::Criteria::meeting()
            .pass_rate(0.5)
            .name("response received")
            .satisfies("response received", |_: &String| Ok(()))
            .build()])
    }
}

/// A service contract whose single criterion fails on every sample.
pub struct FailingServiceContract {
    id: String,
}

impl FailingServiceContract {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl ServiceContract for FailingServiceContract {
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

    fn criteria(&self) -> feotest::criteria::Criteria<String> {
        feotest::criteria::Criteria::of([feotest::criteria::Criteria::meeting()
            .pass_rate(0.5)
            .name("never satisfied")
            .satisfies("never satisfied", |_: &String| {
                Err(feotest::model::ContractViolation::new(
                    "forced",
                    "always fails",
                ))
            })
            .build()])
    }
}

/// A service contract that echoes its input and whose single criterion fails
/// whenever the echoed output is the literal `"fail"`. Driving it with an input
/// mix of `"fail"` / `"ok"` reproduces a controlled pass rate.
pub struct InputJudgedContract {
    id: String,
}

impl InputJudgedContract {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl ServiceContract for InputJudgedContract {
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

    fn criteria(&self) -> feotest::criteria::Criteria<String> {
        feotest::criteria::Criteria::of([feotest::criteria::Criteria::meeting()
            .pass_rate(0.5)
            .name("response acceptable")
            .satisfies("response acceptable", |output: &String| {
                if output == "fail" {
                    Err(feotest::model::ContractViolation::new(
                        "response acceptable",
                        "forced",
                    ))
                } else {
                    Ok(())
                }
            })
            .build()])
    }
}

// ---------------------------------------------------------------------------
// Baseline helpers
// ---------------------------------------------------------------------------

/// Runs a measure experiment against an always-pass contract and returns the
/// temp directory (keeps it alive).
pub fn establish_baseline(service_contract_id: &str, samples: u32) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];
    let id = service_contract_id.to_owned();

    feotest::experiment::MeasureExperiment::builder()
        .service_contract_id(service_contract_id)
        .service_contract(move || SimpleServiceContract::new(id.clone()))
        .samples(samples)
        .inputs(&inputs)
        .baseline_dir(dir.path())
        .build()
        .run();

    dir
}

/// Runs a threshold-first test against a pre-established baseline directory.
///
/// Sets `threshold_origin` to `Sla` so that the explicit threshold does not
/// conflict with the baseline spec (the validation rule rejects `Unspecified`
/// origin when a baseline exists).
pub fn run_against_baseline(
    service_contract_id: &str,
    baseline_dir: &Path,
    samples: u32,
    min_pass_rate: f64,
) -> feotest::ptest::ProbabilisticTestResult {
    let inputs = vec!["input".to_string()];
    ProbabilisticTest::for_contract(SimpleServiceContract::new(service_contract_id))
        .inputs(&inputs)
        .approach(ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        })
        .threshold_origin(feotest::model::ThresholdOrigin::Sla)
        .spec_resolver(SpecResolver::with_dir(baseline_dir))
        .run()
}
