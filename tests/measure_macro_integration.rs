//! Integration tests for the `#[measure_experiment]` macro.

use feotest::measure_experiment;
use feotest::model::TrialOutcome;
use std::time::Duration;

// --- Basic usage ---

#[measure_experiment(use_case = "basic-test", samples = 20)]
fn measure_basic(input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

// --- No input parameter ---

#[measure_experiment(use_case = "no-input", samples = 10)]
fn measure_no_input() -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

// --- With custom inputs ---

#[measure_experiment(
    use_case = "custom-inputs",
    samples = 15,
    inputs = ["alpha", "beta", "gamma"]
)]
fn measure_with_inputs(input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

// --- With experiment ID ---

#[measure_experiment(use_case = "with-id", samples = 10, experiment_id = "baseline-v1")]
fn measure_with_experiment_id(input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

// --- With warmup ---

#[measure_experiment(use_case = "with-warmup", samples = 10, warmup = 3)]
fn measure_with_warmup(input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

// --- With spec_dir (writes to temp dir via env override) ---

#[measure_experiment(use_case = "with-spec", samples = 10, spec_dir = "target/test-specs")]
fn measure_with_spec_dir(input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}

// --- With all optional attributes ---

#[measure_experiment(
    use_case = "full-config",
    samples = 10,
    inputs = ["x", "y"],
    experiment_id = "full-v1",
    warmup = 2,
    time_budget = "30s",
    token_budget = 100000,
    pacing = "50/s"
)]
fn measure_full_config(input: &str) -> TrialOutcome {
    TrialOutcome::success(Duration::from_millis(1))
}
