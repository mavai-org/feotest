//! Latency testing dimension.
//!
//! This module hosts the domain-level machinery for the latency dimension
//! of a probabilistic test: threshold declaration, resolution against a
//! baseline, and the verdict dimension. Whether the dimension binds the
//! test is the run's choice ([`AssertionEnforcement`](crate::verdict::AssertionEnforcement)),
//! never a property of a constraint. The decision
//! rules (`latency/compliance-exact-binomial` for an explicit requirement,
//! `latency/precedence` for a baseline-derived threshold) and the latency
//! gates live in `crate::statistics::latency` and are exercised by the
//! conformance suite.

pub mod criterion;
pub mod dimension;
pub mod percentile;
pub mod resolver;
pub mod thresholds;

pub use criterion::LatencyCriterion;
pub use dimension::{EvaluationStatus, LatencyDimension, LatencyEvaluation};
pub use percentile::Percentile;
pub use resolver::{
    ConstraintConfidence, ConstraintSource, ResolvedLatencyConstraint, ThresholdProvenance, resolve,
};
pub use thresholds::LatencyThresholds;

/// Default confidence level of a latency decision when none is supplied —
/// for an explicit requirement and for a baseline-derived threshold alike.
pub const DEFAULT_LATENCY_CONFIDENCE: f64 = 0.95;
