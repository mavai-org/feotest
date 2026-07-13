//! Probabilistic test execution.
//!
//! A probabilistic test runs a service contract repeatedly, applies statistical
//! inference to the observed outcomes, and produces a verdict: does the
//! service meet its threshold?
//!
//! This module consumes the output of experiments (baseline specs) and the
//! machinery of the statistics module (threshold derivation, evaluation,
//! feasibility checking) to produce [`VerdictRecord`]s.
//!
//! Four operational approaches are supported:
//!
//! | Approach | User specifies | Framework computes |
//! |---|---|---|
//! | **Sample-size-first** | `samples` + `threshold_confidence` | `min_pass_rate` |
//! | **Confidence-first** | `confidence` + `min_detectable_effect` + `power` | `samples` |
//! | **Threshold-first** | `samples` + `min_pass_rate` | implied confidence |
//! | **Risk-driven** | `minimum_acceptable_rate` + `confidence` + `target_power` | `samples` + `min_pass_rate` |
//!
//! [`VerdictRecord`]: crate::verdict::VerdictRecord

mod approach;
mod baseline;
pub mod builder;
mod contract;
mod diagnostics;
mod disclosure;
mod probabilistic_test;
mod runner;

pub use contract::ContractTest;
pub use probabilistic_test::ProbabilisticTest;
pub use runner::ProbabilisticTestResult;

pub mod validation_api {
    //! Re-exports for the proc-macro's generated code.
    pub use super::builder::ThresholdApproach;
}
