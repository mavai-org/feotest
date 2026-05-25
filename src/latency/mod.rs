//! Latency testing dimension.
//!
//! This module hosts the domain-level machinery for the latency dimension
//! of a probabilistic test: threshold declaration, resolution against a
//! baseline, enforcement policy, and the verdict dimension. The underlying
//! statistical primitives (nearest-rank percentile, non-parametric binomial
//! threshold, minimum-sample feasibility) live in `crate::statistics::latency`
//! and are exercised by the conformance suite.

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
pub use resolver::{ResolvedLatencyThreshold, ThresholdProvenance, resolve};
pub use thresholds::LatencyThresholds;

/// Default baseline-derivation confidence level used when none is supplied.
pub const DEFAULT_BASELINE_CONFIDENCE: f64 = 0.95;
