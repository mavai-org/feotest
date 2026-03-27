//! Core types for the statistics module.

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

    /// Returns the significance level α = 1 − confidence.
    #[must_use]
    pub fn alpha(self) -> f64 {
        1.0 - self.0
    }
}

// ---------------------------------------------------------------------------
// OperationalApproach
// ---------------------------------------------------------------------------

/// The strategy used to derive a pass/fail threshold.
///
/// Each variant fixes two of the three variables (sample size, confidence,
/// threshold) and derives the third.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationalApproach {
    /// Fix sample size and confidence; derive threshold from the Wilson lower
    /// bound. Cost-driven: "I can afford N trials."
    SampleSizeFirst,
    /// Fix confidence, effect size, and power; derive the required sample
    /// size. Quality-driven: "I need to detect a Δ drop."
    ConfidenceFirst,
    /// Fix sample size and an explicit threshold; derive the implied
    /// confidence. Baseline-anchored: "I know the threshold I want."
    ThresholdFirst,
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
    /// Bounds are clamped to [0, 1].
    pub(in crate::statistics) const fn new(
        point_estimate: f64,
        sample_size: u32,
        lower_bound: f64,
        upper_bound: f64,
        confidence_level: ConfidenceLevel,
    ) -> Self {
        Self {
            point_estimate,
            sample_size,
            lower_bound: lower_bound.clamp(0.0, 1.0),
            upper_bound: upper_bound.clamp(0.0, 1.0),
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
// DerivationContext
// ---------------------------------------------------------------------------

/// The parameters used to derive a threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivationContext {
    /// Baseline success rate `p̂_baseline`, in [0, 1].
    baseline_rate: f64,
    /// Number of baseline trials.
    baseline_samples: u32,
    /// Number of test trials.
    test_samples: u32,
    /// Confidence level used for derivation.
    confidence: ConfidenceLevel,
}

impl DerivationContext {
    /// Creates a new `DerivationContext`.
    ///
    /// # Panics
    ///
    /// Panics if `baseline_rate` is not in [0, 1] or either sample size is
    /// zero.
    #[must_use]
    pub fn new(
        baseline_rate: f64,
        baseline_samples: u32,
        test_samples: u32,
        confidence: ConfidenceLevel,
    ) -> Self {
        assert!(
            (0.0..=1.0).contains(&baseline_rate),
            "baseline_rate must be in [0, 1], got {baseline_rate}"
        );
        assert!(baseline_samples > 0, "baseline_samples must be positive");
        assert!(test_samples > 0, "test_samples must be positive");
        Self {
            baseline_rate,
            baseline_samples,
            test_samples,
            confidence,
        }
    }

    /// Baseline success rate.
    #[must_use]
    pub const fn baseline_rate(&self) -> f64 {
        self.baseline_rate
    }

    /// Number of baseline trials.
    #[must_use]
    pub const fn baseline_samples(&self) -> u32 {
        self.baseline_samples
    }

    /// Number of test trials.
    #[must_use]
    pub const fn test_samples(&self) -> u32 {
        self.test_samples
    }

    /// Confidence level used for derivation.
    #[must_use]
    pub const fn confidence(&self) -> ConfidenceLevel {
        self.confidence
    }
}

// ---------------------------------------------------------------------------
// DerivedThreshold
// ---------------------------------------------------------------------------

/// A pass/fail threshold derived from baseline data.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedThreshold {
    /// The threshold value in [0, 1].
    value: f64,
    /// Which operational approach produced this threshold.
    approach: OperationalApproach,
    /// The parameters used during derivation.
    context: DerivationContext,
    /// Whether the derivation is considered statistically sound.
    is_statistically_sound: bool,
}

impl DerivedThreshold {
    /// Creates a new `DerivedThreshold`.
    ///
    /// # Panics
    ///
    /// Panics if `value` is not in [0, 1].
    #[must_use]
    pub fn new(
        value: f64,
        approach: OperationalApproach,
        context: DerivationContext,
        is_statistically_sound: bool,
    ) -> Self {
        assert!(
            (0.0..=1.0).contains(&value),
            "threshold must be in [0, 1], got {value}"
        );
        Self {
            value,
            approach,
            context,
            is_statistically_sound,
        }
    }

    /// The threshold value.
    #[must_use]
    pub const fn value(&self) -> f64 {
        self.value
    }

    /// The operational approach used.
    #[must_use]
    pub const fn approach(&self) -> OperationalApproach {
        self.approach
    }

    /// The derivation context.
    #[must_use]
    pub const fn context(&self) -> &DerivationContext {
        &self.context
    }

    /// Whether the threshold is considered statistically sound.
    #[must_use]
    pub const fn is_statistically_sound(&self) -> bool {
        self.is_statistically_sound
    }

    /// The gap between the baseline rate and this threshold.
    ///
    /// A positive value means the threshold is below the baseline.
    #[must_use]
    pub fn gap_from_baseline(&self) -> f64 {
        self.context.baseline_rate - self.value
    }
}

// ---------------------------------------------------------------------------
// SampleSizeRequirement
// ---------------------------------------------------------------------------

/// The result of a power-based sample size calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleSizeRequirement {
    /// The computed minimum sample size.
    required_samples: u32,
    /// Confidence level (1 − α).
    confidence: ConfidenceLevel,
    /// Statistical power (1 − β).
    power: f64,
    /// Minimum detectable effect δ.
    min_detectable_effect: f64,
    /// Baseline (null hypothesis) rate p₀.
    null_rate: f64,
    /// Alternative rate p₁ = p₀ − δ.
    alternative_rate: f64,
}

impl SampleSizeRequirement {
    /// Creates a new `SampleSizeRequirement`.
    pub(in crate::statistics) const fn new(
        required_samples: u32,
        confidence: ConfidenceLevel,
        power: f64,
        min_detectable_effect: f64,
        null_rate: f64,
        alternative_rate: f64,
    ) -> Self {
        Self {
            required_samples,
            confidence,
            power,
            min_detectable_effect,
            null_rate,
            alternative_rate,
        }
    }

    /// The computed minimum sample size.
    #[must_use]
    pub const fn required_samples(&self) -> u32 {
        self.required_samples
    }

    /// Confidence level (1 − α).
    #[must_use]
    pub const fn confidence(&self) -> ConfidenceLevel {
        self.confidence
    }

    /// Statistical power (1 − β).
    #[must_use]
    pub const fn power(&self) -> f64 {
        self.power
    }

    /// Minimum detectable effect δ.
    #[must_use]
    pub const fn min_detectable_effect(&self) -> f64 {
        self.min_detectable_effect
    }

    /// Baseline (null hypothesis) rate p₀.
    #[must_use]
    pub const fn null_rate(&self) -> f64 {
        self.null_rate
    }

    /// Alternative rate p₁ = p₀ − δ.
    #[must_use]
    pub const fn alternative_rate(&self) -> f64 {
        self.alternative_rate
    }
}

// ---------------------------------------------------------------------------
// VerdictWithConfidence
// ---------------------------------------------------------------------------

/// The result of evaluating test outcomes against a derived threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct VerdictWithConfidence {
    /// Whether the test passed (observed rate ≥ threshold).
    passed: bool,
    /// The observed success rate in the test run.
    observed_rate: f64,
    /// The threshold that was applied.
    threshold: DerivedThreshold,
    /// Probability of a false positive (Type I error).
    false_positive_probability: f64,
}

impl VerdictWithConfidence {
    /// Creates a new `VerdictWithConfidence`.
    pub(in crate::statistics) const fn new(
        passed: bool,
        observed_rate: f64,
        threshold: DerivedThreshold,
        false_positive_probability: f64,
    ) -> Self {
        Self {
            passed,
            observed_rate,
            threshold,
            false_positive_probability,
        }
    }

    /// Whether the test passed.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }

    /// The observed success rate.
    #[must_use]
    pub const fn observed_rate(&self) -> f64 {
        self.observed_rate
    }

    /// The threshold that was applied.
    #[must_use]
    pub const fn threshold(&self) -> &DerivedThreshold {
        &self.threshold
    }

    /// Probability of a false positive.
    #[must_use]
    pub const fn false_positive_probability(&self) -> f64 {
        self.false_positive_probability
    }

    /// How far below the threshold the observed rate fell (0 if passed).
    #[must_use]
    pub fn shortfall(&self) -> f64 {
        if self.passed {
            0.0
        } else {
            self.threshold.value() - self.observed_rate
        }
    }

    /// Confidence in the result: 1 − false positive probability.
    #[must_use]
    pub fn confidence(&self) -> f64 {
        1.0 - self.false_positive_probability
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
