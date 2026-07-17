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
mod judgement;
mod measure;
mod optimize;

pub use engine::{ContractExecutionResult, ExecutionEngine, ExecutionResult, SampleEvaluation};
pub use explore::{ConfigResult, ExploreExperiment, ExploreResult};
pub use judgement::{JudgementState, NormativeJudgement};
pub use measure::MeasureExperiment;
pub use optimize::{
    FactorMutator, IterationObservation, IterationRecord, Objective, ObservedPassRate,
    OptimizeExperiment, OptimizeResult, Scorer, TerminationReason,
};
