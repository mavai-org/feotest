//! Default values for statistical parameters.

/// Default confidence level (1 − α) used when none is specified.
pub const DEFAULT_CONFIDENCE: f64 = 0.95;

/// Default significance level (α), derived as 1 − [`DEFAULT_CONFIDENCE`].
pub const DEFAULT_ALPHA: f64 = 1.0 - DEFAULT_CONFIDENCE;

/// Default target power used when the run declared none.
///
/// The probability of detecting a genuine degradation; 0.80 is the
/// conventional choice. The report's sensitivity statements are made at
/// this power for runs whose sizing did not declare one.
pub const DEFAULT_TARGET_POWER: f64 = 0.80;

/// Minimum confidence level below which a derived threshold is flagged as
/// statistically unsound.
pub(in crate::statistics) const SOUNDNESS_FLOOR: f64 = 0.80;
