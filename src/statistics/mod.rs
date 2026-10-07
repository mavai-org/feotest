//! Statistical inference for Bernoulli trial outcomes and latencies.
//!
//! This module holds the decision rules of the Statistical Companion,
//! methodology 1.6.0 — each an exact finite computation, named by a
//! versioned identifier that travels with every verdict it decides — and the
//! descriptive statistics reported beside them.
//!
//! # Module structure
//!
//! - [`rules`] — the decision rules, configuration errors and test intent
//! - [`regression`] — `regression/fisher`: the one-sided Fisher cutoff, its
//!   size and powers, and the threshold-first inversion
//! - [`compliance`] — `compliance/exact-binomial`: the smallest passing
//!   count, feasibility and compliance sizing
//! - [`latency`] — `latency/precedence` and
//!   `latency/compliance-exact-binomial`, percentiles and the latency gates
//! - [`decision`] — verdicts, one criterion under its rule, and the
//!   structural composition into the test verdict
//! - [`risk_driven_sizing`] — design and resolved sizing of a regression
//!   test
//! - [`feasibility`] — whether a normative design can pass at all
//! - [`exact`] — the exact-boundary convention
//! - [`proportion`] — Wilson score intervals (descriptive only; no rule
//!   decides with them)
//! - [`types`] — shared domain types
//! - [`defaults`] — default statistical parameters

pub mod compliance;
pub mod decision;
pub mod defaults;
mod distributions;
pub mod exact;
pub mod feasibility;
pub mod latency;
pub mod proportion;
pub mod regression;
pub mod risk_driven_sizing;
pub mod rules;
pub mod types;
