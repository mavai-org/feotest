//! Probabilistic test execution.
//!
//! A probabilistic test runs a service contract repeatedly, applies statistical
//! inference to the observed outcomes, and produces a verdict: does the
//! service meet its threshold?
//!
//! This module consumes the output of experiments (baseline specs) and the
//! decision rules of the statistics module to produce [`VerdictRecord`]s.
//! A configuration with any invalid part is refused whole before any sample
//! runs, and the refusal is itself recorded.
//!
//! Four operational approaches are supported:
//!
//! | Approach | User specifies | Framework computes |
//! |---|---|---|
//! | **Sample-size-first** | `samples` + `confidence` | each regression cutoff |
//! | **Confidence-first** | `confidence` + `min_detectable_effect` + `power` | `samples` (resolved sizing) |
//! | **Threshold-first** | `samples` + `min_pass_rate` | the implied alpha of the cutoff |
//! | **Risk-driven** | `design_alternative_rate` + `confidence` + `target_power` | `samples` (resolved sizing) |
//!
//! [`VerdictRecord`]: crate::verdict::VerdictRecord

mod approach;
mod baseline;
pub mod builder;
mod contract;
mod diagnostics;
mod disclosure;
mod judge;
mod preflight;
mod probabilistic_test;
mod runner;

pub use contract::ContractTest;
pub use probabilistic_test::ProbabilisticTest;
pub use runner::ProbabilisticTestResult;

pub mod validation_api {
    //! Re-exports for the proc-macro's generated code.
    pub use super::builder::ThresholdApproach;
}
