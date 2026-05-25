//! Integration tests for the `#[probabilistic_test]` macro.

use feotest::probabilistic_test;

// --- Threshold-first approach ---

#[probabilistic_test(samples = 50, threshold = 0.80)]
fn threshold_first_always_passes(_input: &str) -> bool {
    true
}

#[probabilistic_test(samples = 30, threshold = 0.90, intent = "smoke")]
fn threshold_first_with_smoke_intent(_input: &str) -> bool {
    true
}

#[probabilistic_test(
    samples = 60,
    threshold = 0.95,
    threshold_origin = "sla",
    contract_ref = "API SLA v2.3 §4.1"
)]
fn threshold_first_with_provenance(_input: &str) -> bool {
    true
}

#[probabilistic_test(samples = 40, threshold = 0.80)]
fn threshold_first_plain(_input: &str) -> bool {
    true
}

// --- No-input variant ---

#[probabilistic_test(samples = 30, threshold = 0.90)]
fn threshold_first_no_input_param() -> bool {
    true
}

// --- Threshold-first: verify actual statistical behaviour ---

#[probabilistic_test(samples = 50, threshold = 0.50)]
fn threshold_first_high_pass_rate(_input: &str) -> bool {
    // Always succeeds — well above 50% threshold
    true
}

// --- Sample-size-first approach ---

#[probabilistic_test(
    samples = 50,
    confidence = 0.95,
    spec = "tests/fixtures/test-baseline.yaml"
)]
fn sample_size_first_with_spec(_input: &str) -> bool {
    true
}

#[probabilistic_test(
    samples = 50,
    confidence = 0.95,
    spec = "tests/fixtures/test-baseline.yaml",
    threshold_origin = "empirical"
)]
fn sample_size_first_with_origin(_input: &str) -> bool {
    true
}

// --- Result-returning body (passes on Ok) ---

#[probabilistic_test(samples = 40, threshold = 0.80)]
fn result_body_passes_on_ok(input: &str) -> Result<(), feotest::model::ContractViolation> {
    if input.is_empty() {
        Err(feotest::model::ContractViolation::new("empty", "no input"))
    } else {
        Ok(())
    }
}
