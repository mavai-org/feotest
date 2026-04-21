//! Domain types for trials, outcomes, and sample aggregates.
//!
//! These types form the shared vocabulary of the framework. They are small,
//! explicit, and carry no behaviour beyond construction and access.

mod outcome;
mod sample;
mod types;

pub use outcome::{ContractViolation, Outcome, TrialOutcome};
pub use sample::SampleAggregate;
pub use types::{
    BudgetExhaustedBehavior, CostSummary, ExceptionHandling, ExecutionSummary, ExpirationInfo,
    ExpirationStatus, PacingSummary, RunScopedSnapshot, TerminationInfo, TerminationReason,
    TestIdentity, TestIntent, ThresholdOrigin, Warning,
};
