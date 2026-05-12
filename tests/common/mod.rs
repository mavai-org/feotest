//! Shared helpers for integration tests.

use std::path::Path;
use std::time::Duration;

use feotest::model::TrialOutcome;
use feotest::ptest::ProbabilisticTestBuilder;
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
// Use case helpers
// ---------------------------------------------------------------------------

/// A simple use case with no covariates.
pub struct SimpleServiceContract {
    id: String,
}

impl SimpleServiceContract {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl ServiceContract for SimpleServiceContract {
    fn id(&self) -> &str {
        &self.id
    }
}

// ---------------------------------------------------------------------------
// Baseline helpers
// ---------------------------------------------------------------------------

/// Runs a measure experiment and returns the temp directory (keeps it alive).
pub fn establish_baseline(
    service_contract_id: &str,
    samples: u32,
    trial: impl Fn(&str) -> TrialOutcome + 'static,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let inputs = vec!["input".to_string()];

    feotest::experiment::MeasureExperiment::builder()
        .service_contract_id(service_contract_id)
        .service_contract(|| ())
        .samples(samples)
        .inputs(&inputs)
        .trial(move |(): &(), input| trial(input))
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
    trial: impl FnMut(&str) -> TrialOutcome,
) -> feotest::ptest::ProbabilisticTestResult {
    let inputs = vec!["input".to_string()];
    ProbabilisticTestBuilder::new(service_contract_id, &inputs, trial)
        .approach(ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        })
        .threshold_origin(feotest::model::ThresholdOrigin::Sla)
        .spec_resolver(SpecResolver::with_dir(baseline_dir))
        .run()
}
