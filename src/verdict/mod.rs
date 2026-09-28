//! Verdict logic: mapping statistical results to pass/fail decisions.
//!
//! A verdict combines an observed pass rate, a required threshold, and a
//! statistical confidence bound into a final determination of whether the
//! system under test meets its specification.
//!
//! [`VerdictRecord`] is the single source of truth consumed by all rendering
//! paths: machine-readable XML, human-readable HTML reports, and console output.

mod assessment;
mod record;

pub use assessment::{CriterionRow, FunctionalAssessment};
pub use record::{
    BaselineProvenance, ComplianceEvidence, CovariateStatus, DesignDisclosure, Misalignment,
    RegressionEvidence, RuleEvidence, SpecProvenance, StatisticalAnalysis, VerdictRecord,
    VerdictRecordBuilder,
};

pub use crate::statistics::decision::Verdict;
