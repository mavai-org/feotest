//! Statistical inference for Bernoulli trial outcomes.
//!
//! This module provides the core statistical machinery: confidence interval
//! computation, threshold derivation from empirical baselines, and hypothesis
//! testing for pass-rate claims.
//!
//! The primary model is a sequence of independent Bernoulli trials with a
//! common success probability. Verdicts are derived from one-sided confidence
//! bounds rather than naïve point estimates.
//!
//! # Module structure
//!
//! - [`proportion`] — Wilson score confidence intervals and z-tests
//! - [`threshold`] — deriving pass/fail thresholds from baseline data
//! - [`sample_size`] — power analysis for sample size planning
//! - [`risk_driven_sizing`] — sample sizing against the moving acceptance floor
//! - [`evaluator`] — evaluating test outcomes against thresholds
//! - [`feasibility`] — pre-flight checks on sample sizing
//! - [`latency`] — empirical percentiles and latency threshold derivation
//! - [`types`] — shared domain types
//! - [`defaults`] — default statistical parameters

pub mod defaults;
pub mod evaluator;
pub mod feasibility;
pub mod latency;
pub mod proportion;
pub mod risk_driven_sizing;
pub mod sample_size;
pub mod threshold;
pub mod types;
