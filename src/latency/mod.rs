//! Latency testing dimension.
//!
//! This module hosts the domain-level machinery for the latency dimension
//! of a probabilistic test: threshold declaration, resolution against a
//! baseline, enforcement policy, and the verdict dimension. The decision
//! rules (`latency/compliance-exact-binomial` for an explicit requirement,
//! `latency/precedence` for a baseline-derived threshold) and the latency
//! gates live in `crate::statistics::latency` and are exercised by the
//! conformance suite.

pub mod criterion;
pub mod dimension;
pub mod enforcement;
pub mod percentile;
pub mod resolver;
pub mod thresholds;

pub use criterion::LatencyCriterion;
pub use dimension::{EvaluationStatus, LatencyDimension, LatencyEvaluation};
pub use enforcement::{LatencyEnforcementMode, resolved_mode_from_env};
pub use percentile::Percentile;
pub use resolver::{
    ConstraintConfidence, ConstraintSource, ResolvedLatencyConstraint, ThresholdProvenance, resolve,
};
pub use thresholds::LatencyThresholds;

/// Default confidence level of a latency decision when none is supplied —
/// for an explicit requirement and for a baseline-derived threshold alike.
pub const DEFAULT_LATENCY_CONFIDENCE: f64 = 0.95;
