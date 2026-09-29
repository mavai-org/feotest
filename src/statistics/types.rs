//! Core types for the statistics module.

/// Maximum magnitude of floating-point undershoot (below 0.0) or overshoot
/// (above 1.0) tolerated in Wilson score interval bounds. The Wilson formula
/// can produce bounds fractionally outside [0, 1] due to IEEE 754 rounding;
/// values within this tolerance are snapped to the nearest boundary. Values
/// outside it indicate a computational error and trigger a panic.
const BOUND_TOLERANCE: f64 = 0.001;

/// Snaps a confidence interval bound to [0, 1], tolerating floating-point
/// noise up to [`BOUND_TOLERANCE`]. Panics if the value is further out.
fn snap_bound(value: f64, name: &str) -> f64 {
    if (0.0..=1.0).contains(&value) {
        return value;
    }
    if (-BOUND_TOLERANCE..0.0).contains(&value) {
        return 0.0;
    }
    if value > 1.0 && value <= 1.0 + BOUND_TOLERANCE {
        return 1.0;
    }
    panic!("{name} is {value}, which is more than {BOUND_TOLERANCE} outside [0, 1]");
}

// ---------------------------------------------------------------------------
// ConfidenceLevel newtype
// ---------------------------------------------------------------------------

/// A confidence level in the open interval (0, 1).
///
/// Wraps an `f64` and guarantees at construction time that the value lies
/// strictly between 0 and 1.
///
/// # Panics
///
/// Construction panics if the value is not in (0, 1). An out-of-range
/// confidence level is a programming error, not a runtime condition.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ConfidenceLevel(f64);

impl ConfidenceLevel {
    /// Creates a new `ConfidenceLevel`.
    ///
    /// # Panics
    ///
    /// Panics if `value` is not in the open interval (0, 1).
    #[must_use]
    pub fn new(value: f64) -> Self {
        assert!(
            value > 0.0 && value < 1.0,
            "confidence level must be in (0, 1), got {value}"
        );
        Self(value)
    }

    /// Returns the inner `f64` value.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }

    /// Returns the significance level α = 1 − confidence, as the decimal
    /// it is written as (`0.95` gives exactly `0.05`).
    #[must_use]
    pub fn alpha(self) -> f64 {
        crate::statistics::rules::alpha_from_confidence(self.0)
    }
}

// ---------------------------------------------------------------------------
// ProportionEstimate
// ---------------------------------------------------------------------------

/// A two-sided Wilson score confidence interval for a binomial proportion.
#[derive(Debug, Clone, PartialEq)]
pub struct ProportionEstimate {
    /// Point estimate p̂ = successes / trials, in [0, 1].
    point_estimate: f64,
    /// Number of trials.
    sample_size: u32,
    /// Lower bound of the Wilson score interval, in [0, 1].
    lower_bound: f64,
    /// Upper bound of the Wilson score interval, in [0, 1].
    upper_bound: f64,
    /// The confidence level used to compute this interval.
    confidence_level: ConfidenceLevel,
}

impl ProportionEstimate {
    /// Creates a new `ProportionEstimate`.
    ///
    /// Bounds within [`BOUND_TOLERANCE`] of [0, 1] are snapped to the
    /// nearest boundary to absorb floating-point noise from the Wilson
    /// score formula. Bounds outside this tolerance are programming errors.
    ///
    /// # Panics
    ///
    /// Panics if either bound is more than [`BOUND_TOLERANCE`] outside [0, 1].
    pub(in crate::statistics) fn new(
        point_estimate: f64,
        sample_size: u32,
        lower_bound: f64,
        upper_bound: f64,
        confidence_level: ConfidenceLevel,
    ) -> Self {
        Self {
            point_estimate,
            sample_size,
            lower_bound: snap_bound(lower_bound, "lower_bound"),
            upper_bound: snap_bound(upper_bound, "upper_bound"),
            confidence_level,
        }
    }

    /// Point estimate p̂ = successes / trials.
    #[must_use]
    pub const fn point_estimate(&self) -> f64 {
        self.point_estimate
    }

    /// Number of trials.
    #[must_use]
    pub const fn sample_size(&self) -> u32 {
        self.sample_size
    }

    /// Lower bound of the confidence interval.
    #[must_use]
    pub const fn lower_bound(&self) -> f64 {
        self.lower_bound
    }

    /// Upper bound of the confidence interval.
    #[must_use]
    pub const fn upper_bound(&self) -> f64 {
        self.upper_bound
    }

    /// The confidence level used.
    #[must_use]
    pub const fn confidence_level(&self) -> ConfidenceLevel {
        self.confidence_level
    }

    /// Width of the confidence interval: upper − lower.
    #[must_use]
    pub fn interval_width(&self) -> f64 {
        self.upper_bound - self.lower_bound
    }

    /// Half the interval width.
    #[must_use]
    pub fn margin_of_error(&self) -> f64 {
        self.interval_width() / 2.0
    }
}

// ---------------------------------------------------------------------------
// FeasibilityResult
// ---------------------------------------------------------------------------

/// The result of a pre-flight feasibility check.
///
/// Determines whether a configured sample size can produce
/// verification-grade evidence for a given target proportion.
#[derive(Debug, Clone, PartialEq)]
pub struct FeasibilityResult {
    /// Whether the configured sample size is sufficient.
    feasible: bool,
    /// The minimum sample size needed.
    minimum_samples: u32,
    /// The significance level used.
    configured_alpha: f64,
    /// The target proportion being verified.
    target: f64,
    /// The sample size as configured.
    configured_samples: u32,
    /// Description of the statistical method used.
    criterion: String,
}

impl FeasibilityResult {
    /// Creates a new `FeasibilityResult`.
    pub(in crate::statistics) const fn new(
        feasible: bool,
        minimum_samples: u32,
        configured_alpha: f64,
        target: f64,
        configured_samples: u32,
        criterion: String,
    ) -> Self {
        Self {
            feasible,
            minimum_samples,
            configured_alpha,
            target,
            configured_samples,
            criterion,
        }
    }

    /// Whether the configured sample size is sufficient.
    #[must_use]
    pub const fn feasible(&self) -> bool {
        self.feasible
    }

    /// The minimum sample size needed.
    #[must_use]
    pub const fn minimum_samples(&self) -> u32 {
        self.minimum_samples
    }

    /// The significance level used.
    #[must_use]
    pub const fn configured_alpha(&self) -> f64 {
        self.configured_alpha
    }

    /// The target proportion being verified.
    #[must_use]
    pub const fn target(&self) -> f64 {
        self.target
    }

    /// The configured sample size.
    #[must_use]
    pub const fn configured_samples(&self) -> u32 {
        self.configured_samples
    }

    /// Description of the statistical method used.
    #[must_use]
    pub fn criterion(&self) -> &str {
        &self.criterion
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_bound_passes_valid_values() {
        assert!((snap_bound(0.0, "lb") - 0.0).abs() < f64::EPSILON);
        assert!((snap_bound(0.5, "lb") - 0.5).abs() < f64::EPSILON);
        assert!((snap_bound(1.0, "ub") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn snap_bound_rounds_small_undershoot_to_zero() {
        assert!((snap_bound(-0.0005, "lb") - 0.0).abs() < f64::EPSILON);
        assert!((snap_bound(-0.001, "lb") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn snap_bound_rounds_small_overshoot_to_one() {
        assert!((snap_bound(1.0005, "ub") - 1.0).abs() < f64::EPSILON);
        assert!((snap_bound(1.001, "ub") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    #[should_panic(expected = "outside [0, 1]")]
    fn snap_bound_panics_on_large_undershoot() {
        snap_bound(-0.002, "lower_bound");
    }

    #[test]
    #[should_panic(expected = "outside [0, 1]")]
    fn snap_bound_panics_on_large_overshoot() {
        snap_bound(1.002, "upper_bound");
    }

    #[test]
    fn proportion_estimate_accepts_clean_bounds() {
        let cl = ConfidenceLevel::new(0.95);
        let est = ProportionEstimate::new(0.9, 100, 0.85, 0.95, cl);
        assert!((est.lower_bound() - 0.85).abs() < f64::EPSILON);
        assert!((est.upper_bound() - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn proportion_estimate_snaps_tiny_undershoot() {
        let cl = ConfidenceLevel::new(0.95);
        let est = ProportionEstimate::new(0.01, 5, -0.0003, 0.05, cl);
        assert!((est.lower_bound() - 0.0).abs() < f64::EPSILON);
    }
}
