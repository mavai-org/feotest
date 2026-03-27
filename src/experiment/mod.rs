//! Experiment workflows and the common execution engine.
//!
//! All experiment types and probabilistic tests share a single execution engine
//! that handles warmup, input cycling, budget enforcement, pacing, and early
//! termination.
//!
//! Three experiment types build on this engine:
//! - [`MeasureExperiment`] — large-sample baseline establishment
//! - [`ExploreExperiment`] — rapid configuration comparison
//! - [`OptimizeExperiment`] — iterative factor tuning

mod engine;
mod explore;
mod measure;
mod optimize;

pub use engine::{ExecutionEngine, ExecutionResult};
pub use explore::ExploreExperiment;
pub use measure::MeasureExperiment;
pub use optimize::{FactorMutator, Objective, OptimizeExperiment, Scorer};
