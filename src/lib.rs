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
//! - **`controls`** — operational safeguards: warm-up, budgets, catastrophic
//!   outcome handling.
//! - **`experiment`** — experiment workflows for establishing empirical baselines.
//! - **`ptest`** — probabilistic test execution and verdict production.
//! - **`reporting`** — structured output of verdicts and diagnostics.
//! - **`sentinel`** — reliability specifications: structs that aggregate
//!   probabilistic tests and experiments for one non-deterministic boundary,
//!   discoverable at link time.
//! - **`usecase`** — the unit of work under test: a named service invocation
//!   with associated configuration.
//!
//! The initial focus is on a correct, well-tested statistics and inference core.
//! Runner integration and ergonomic test macros will follow once the foundation
//! is solid.

// Self-alias so proc-macros that reference `::feotest::...` resolve correctly
// both inside this crate and in downstream consumers.
extern crate self as feotest;

pub mod controls;
pub mod criteria;
pub mod experiment;
pub mod latency;
pub mod model;
pub mod ptest;
pub mod reporting;
pub mod sentinel;
pub mod spec;
pub mod statistics;
pub mod service_contract;
pub mod verdict;

pub use controls::RunBudget;
pub use feotest_macros::{
    include_baselines, probabilistic_test, sentinel, sentinel_impl, service_contract_factory,
};
pub use model::BudgetExhaustedBehavior;

// Re-exported so the `#[sentinel]` macro can reach `inventory::submit!`
// through the host crate without requiring consumers to add `inventory`
// to their own `Cargo.toml`.
#[doc(hidden)]
pub use inventory;
