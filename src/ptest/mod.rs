//! Probabilistic test execution.
//!
//! A probabilistic test runs a use case repeatedly, applies statistical
//! inference to the observed outcomes, and produces a verdict: does the
//! service meet its threshold?
//!
//! This module consumes the output of experiments (baseline specs) and the
//! machinery of the statistics module (threshold derivation, evaluation,
//! feasibility checking) to produce [`VerdictRecord`]s.
//!
//! Three operational approaches are supported:
//!
//! | Approach | User specifies | Framework computes |
//! |---|---|---|
//! | **Sample-size-first** | `samples` + `threshold_confidence` | `min_pass_rate` |
//! | **Confidence-first** | `confidence` + `min_detectable_effect` + `power` | `samples` |
//! | **Threshold-first** | `samples` + `min_pass_rate` | implied confidence |
//!
//! [`VerdictRecord`]: crate::verdict::VerdictRecord

pub mod builder;
mod runner;

pub use builder::ProbabilisticTestBuilder;
pub use runner::ProbabilisticTestResult;
