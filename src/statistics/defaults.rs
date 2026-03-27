//! Default values for statistical parameters.

/// Default confidence level (1 − α) used when none is specified.
pub const DEFAULT_CONFIDENCE: f64 = 0.95;

/// Default significance level (α), derived as 1 − [`DEFAULT_CONFIDENCE`].
pub const DEFAULT_ALPHA: f64 = 1.0 - DEFAULT_CONFIDENCE;

/// Minimum confidence level below which a derived threshold is flagged as
/// statistically unsound.
pub(in crate::statistics) const SOUNDNESS_FLOOR: f64 = 0.80;
