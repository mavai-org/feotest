//! Operational safeguards: warm-up, budgets, pacing, and token tracking.
//!
//! Stochastic service testing requires discipline beyond simply running trials.
//! This module provides configuration types for budgets, pacing constraints,
//! and a thread-safe token recorder for dynamic token tracking.

mod config;
mod cost;
pub mod run;
mod tokens;

pub use config::{ExecutionConfig, PacingConfig};
pub use cost::Cost;
pub use run::RunBudget;
pub use tokens::TokenRecorder;
