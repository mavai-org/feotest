//! # feotest
//!
//! A probabilistic testing framework for stochastic services.
//!
//! `feotest` provides statistical inference machinery for determining whether
//! a stochastic service — such as an LLM-backed endpoint, a classifier, or a
//! ranking system — meets a specified quality threshold, based on repeated
//! empirical trials modelled as Bernoulli experiments.
//!
//! ## Design
//!
//! The framework is organised around a small number of core concerns:
//!
//! - **`statistics`** — confidence intervals, threshold derivation, and
//!   inference over Bernoulli trial outcomes.
//! - **`model`** — domain types representing trials, outcomes, and sample
//!   aggregates.
//! - **`verdict`** — the logic that maps statistical results to pass/fail
//!   decisions.
//! - **`spec`** — baseline specifications describing expected service behaviour.
//! - **`contract`** — success/failure criteria for individual service invocations.
//! - **`controls`** — operational safeguards: warm-up, budgets, catastrophic
//!   outcome handling.
//! - **`experiment`** — experiment workflows for establishing empirical baselines.
//! - **`reporting`** — structured output of verdicts and diagnostics.
//! - **`usecase`** — the unit of work under test: a named service invocation
//!   with associated configuration.
//!
//! The initial focus is on a correct, well-tested statistics and inference core.
//! Runner integration and ergonomic test macros will follow once the foundation
//! is solid.

pub mod contract;
pub mod controls;
pub mod experiment;
pub mod model;
pub mod reporting;
pub mod spec;
pub mod statistics;
pub mod usecase;
pub mod verdict;
