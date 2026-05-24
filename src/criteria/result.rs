//! The per-sample evaluation result for a single criterion.

use crate::model::ContractViolation;

/// The two-valued outcome of evaluating one criterion on one sample.
///
/// A criterion either passes cleanly or fails with a reason. A failed
/// transformation — when no testable value could be produced — is a `Fail`,
/// not a third state: it counts in the criterion's denominator like any other
/// failure. The inconclusive distinction is three-valued, but it lives at the
/// aggregate criterion-verdict level, never here at the per-sample level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CriterionOutcome {
    /// Every postcondition held for this sample (and any transform succeeded).
    Pass,
    /// The criterion did not hold for this sample.
    Fail,
}

/// The full per-sample record for one criterion: which criterion it is, its
/// outcome, and — for a `Fail` — the violation that explains it.
///
/// A `Pass` carries no reason. A `Fail` always carries the violation that
/// caused it: either a failing postcondition or a failed transformation,
/// distinguished by the violation's check name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CriterionSampleResult {
    criterion: String,
    outcome: CriterionOutcome,
    reason: Option<ContractViolation>,
}

impl CriterionSampleResult {
    /// A clean pass for the named criterion.
    #[must_use]
    pub fn pass(criterion: impl Into<String>) -> Self {
        Self {
            criterion: criterion.into(),
            outcome: CriterionOutcome::Pass,
            reason: None,
        }
    }

    /// A failure for the named criterion, carrying the violation that caused
    /// it (a failing postcondition or a failed transformation).
    #[must_use]
    pub fn fail(criterion: impl Into<String>, reason: ContractViolation) -> Self {
        Self {
            criterion: criterion.into(),
            outcome: CriterionOutcome::Fail,
            reason: Some(reason),
        }
    }

    /// The criterion this result belongs to.
    #[must_use]
    pub fn criterion(&self) -> &str {
        &self.criterion
    }

    /// The per-sample outcome.
    #[must_use]
    pub const fn outcome(&self) -> CriterionOutcome {
        self.outcome
    }

    /// The violation explaining a `Fail`, or `None` for a `Pass`.
    #[must_use]
    pub const fn reason(&self) -> Option<&ContractViolation> {
        self.reason.as_ref()
    }

    /// Whether this sample passed the criterion.
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(self.outcome, CriterionOutcome::Pass)
    }
}
