//! Integration-test fixture. **Not a pedagogic example.**
//!
//! This file exists solely to drive the end-to-end tests in
//! `tests/sentinel_demo.rs`. Cargo's `examples/` directory is the only
//! place from which those tests can invoke a compiled binary via
//! `cargo run --example sentinel_demo -- <subcommand>`; the file lives
//! here to satisfy that layout constraint, nothing more.
//!
//! Pedagogic examples of sentinel authoring — narrative walkthroughs,
//! tutorial-style code, beginner-oriented scenarios — belong in the
//! sibling `feotest-examples` project, not here. Do not extend this
//! file with teaching material.
//!
//! The two `#[sentinel]` structs below are the minimum needed to cover
//! every runtime-behaviour branch the integration tests assert against:
//! a normative-origin test (no baseline resolution) and an
//! empirical-origin test paired with its measure experiment (baseline
//! resolution chain exercised in both the missing and
//! successfully-resolved states).
//!
//! The invocation shapes the integration tests use:
//!
//! ```text
//! cargo run --example sentinel_demo -- list
//! cargo run --example sentinel_demo -- run <spec>
//! cargo run --example sentinel_demo -- measure --output <uri> <spec>
//! cargo run --example sentinel_demo -- check --baselines <dir>
//! ```

use feotest::sentinel;
use feotest::sentinel_impl;

/// A reliability specification whose sole probabilistic test carries a
/// normative (SLA-origin) threshold and therefore needs no external
/// baseline specification.
#[sentinel(description = "SLA-origin demo spec (no external baseline required)")]
#[derive(Default)]
pub struct SlaDemo;

#[sentinel_impl]
impl SlaDemo {
    /// Always succeeds. A real service would call out here.
    #[probabilistic_test(origin = "sla", threshold = 0.95, samples = 100)]
    const fn always_ok(&self) -> bool {
        // In a real spec this is where the service would be called. The demo
        // keeps the body trivial so integration tests produce stable verdicts.
        let _ = self;
        true
    }
}

/// A reliability specification with an EMPIRICAL-origin probabilistic
/// test paired with a measure experiment. Demonstrates the measure ↔
/// test pairing and the baseline resolution chain.
#[sentinel(description = "Empirical-origin demo spec (requires a baseline)")]
#[derive(Default)]
pub struct EmpiricalDemo;

#[sentinel_impl]
impl EmpiricalDemo {
    #[measure_experiment(baseline_for = "matches_baseline", samples = 100)]
    const fn calibrate(&self) -> bool {
        // Deterministic success rate for reproducible baselines.
        let _ = self;
        true
    }

    #[probabilistic_test(
        origin = "empirical",
        samples = 100,
        confidence = 0.90,
        baseline = "calibrate"
    )]
    const fn matches_baseline(&self) -> bool {
        let _ = self;
        true
    }
}

fn main() -> std::process::ExitCode {
    feotest::sentinel::run_cli()
}
