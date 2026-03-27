//! Operational safeguards: warm-up, budgets, and catastrophic outcome handling.
//!
//! Stochastic service testing requires discipline beyond simply running trials.
//! This module provides mechanisms for warm-up periods, cost and time budgets,
//! and detection of catastrophic outcomes that should halt execution immediately.
