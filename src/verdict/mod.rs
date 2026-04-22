//! Verdict logic: mapping statistical results to pass/fail decisions.
//!
//! A verdict combines an observed pass rate, a required threshold, and a
//! statistical confidence bound into a final determination of whether the
//! system under test meets its specification.
//!
//! [`VerdictRecord`] is the single source of truth consumed by all rendering
//! paths: machine-readable XML, human-readable HTML reports, and console output.

mod record;

pub use record::{
    BaselineProvenance, CovariateStatus, FunctionalDimension, Misalignment, SpecProvenance,
    StatisticalAnalysis, VerdictRecord, VerdictRecordBuilder,
};

use serde::{Serialize, Serializer};
use std::fmt;

/// The outcome of a probabilistic test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Insufficient evidence to reject H0.
    /// No statistically significant divergence from baseline detected.
    Pass,

    /// H0 rejected. Sufficient statistical evidence of divergence
    /// from baseline. This is the call to action.
    Fail,

    /// Statistical analysis cannot be relied upon.
    /// Typically caused by covariate misalignment or insufficient data.
    Inconclusive,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(f, "PASS"),
            Self::Fail => write!(f, "FAIL"),
            Self::Inconclusive => write!(f, "INCONCLUSIVE"),
        }
    }
}

impl Serialize for Verdict {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_display() {
        assert_eq!(Verdict::Pass.to_string(), "PASS");
        assert_eq!(Verdict::Fail.to_string(), "FAIL");
        assert_eq!(Verdict::Inconclusive.to_string(), "INCONCLUSIVE");
    }
}
