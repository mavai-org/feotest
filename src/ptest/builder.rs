//! The threshold-derivation approach and the baseline-resolver helpers shared
//! by the contract-driven probabilistic test.
//!
//! [`ThresholdApproach`] names the four ways a success-rate threshold and a
//! sample count relate; the resolver helpers locate a baseline spec on disk.

use std::path::{Path, PathBuf};

use crate::spec::SpecResolver;

/// Configures the sampling plan and how the regression cutoff is derived.
///
/// Exactly one approach applies to a test. Sample size, confidence, and the
/// detectable degradation are linked: the caller fixes some, the framework
/// derives the rest. Every baseline-derived criterion is decided by
/// `regression/fisher` at the test's own size, and every declared
/// requirement by `compliance/exact-binomial` (Statistical Companion 1.6.0).
#[derive(Debug, Clone)]
// mavai-ref: JVI-0FVFYBM — do not remove (resolves in mavai-orchestrator)
// mavai-ref: JVI-5YJVXGF — do not remove (resolves in mavai-orchestrator)
// mavai-ref: JVI-6789AKT — do not remove (resolves in mavai-orchestrator)
pub enum ThresholdApproach {
    /// Fix samples and confidence; the cutoff of each baseline-derived
    /// criterion is the one-sided Fisher cutoff at that size and level.
    SampleSizeFirst {
        /// Number of test samples.
        samples: u32,
        /// Confidence level (`1 − alpha`) of the decisions.
        confidence: f64,
    },

    /// Fix confidence, the smallest degradation worth detecting, and power;
    /// derive the sample count.
    ///
    /// Each baseline-derived criterion is sized by resolved sizing against
    /// its observed baseline, at the design alternative rate
    /// `baseline rate − min_detectable_effect`; the largest requirement
    /// governs the run. [`RiskDriven`](Self::RiskDriven) is the same sizing
    /// with the design alternative rate stated absolutely.
    ConfidenceFirst {
        /// Confidence level (`1 − alpha`) of the decisions.
        confidence: f64,
        /// Smallest degradation worth detecting (absolute drop in pass
        /// rate).
        min_detectable_effect: f64,
        /// Probability of detecting a degradation of that size.
        power: f64,
    },

    /// Fix samples and a minimum pass rate.
    ///
    /// The rate is the run's early-termination floor; each criterion is
    /// still decided by its own rule. Against a baseline, the report
    /// discloses the threshold-first inversion: the implied alpha of the
    /// cutoff `⌈min_pass_rate · samples⌉` under `regression/fisher`
    /// (Statistical Companion §6.3), flagged unsound above 0.20.
    ThresholdFirst {
        /// Number of test samples.
        samples: u32,
        /// Explicit minimum pass rate.
        min_pass_rate: f64,
    },

    /// Declare a risk appetite; derive the sample count.
    ///
    /// This is the **confidence-first** operational approach in its
    /// risk-driven form: the caller states the **design alternative rate** —
    /// the true success rate at which the test must reach its target power —
    /// and the framework sizes the test by resolved sizing against the
    /// observed baseline (Statistical Companion §5.4.1): the smallest sample
    /// count from which the resolved power stays at or above the target for
    /// every larger test up to the baseline's size. The design alternative
    /// rate is a declared design input, not a tolerance: the test still flags
    /// any degradation from the baseline, including one to a rate above it.
    ///
    /// With several baseline-derived criteria, each criterion is sized
    /// against its own baseline tally and the largest requirement governs
    /// the run.
    ///
    /// Resolving this approach panics if no baseline is available, if the
    /// governing baseline observed no successes, if `design_alternative_rate`
    /// does not sit strictly below a criterion's baseline rate — to demand
    /// more than the baseline delivered, re-measure the baseline rather than
    /// raising the rate — or if no test up to the baseline's size reaches and
    /// holds the target power (a larger baseline is needed).
    RiskDriven {
        /// The true rate at which the test must reach its target power — a
        /// declared design input, not a measured estimate. Must sit strictly
        /// below the baseline rate.
        design_alternative_rate: f64,
        /// Confidence level (`1 − alpha`) of the decisions.
        confidence: f64,
        /// Probability that a service truly at the design alternative rate
        /// fails the test (0.80 is a conventional choice).
        target_power: f64,
    },
}

impl ThresholdApproach {
    /// The canonical operational-approach name, as the methodology states it.
    ///
    /// Risk-driven sizing is not a separate approach: it is the
    /// confidence-first approach priced self-consistently against an
    /// empirical baseline, so it is named `confidence-first (risk-driven)`.
    /// Renderers and disclosures use this name verbatim.
    #[must_use]
    pub const fn canonical_name(&self) -> &'static str {
        match self {
            Self::SampleSizeFirst { .. } => "sample-size-first",
            Self::ConfidenceFirst { .. } => "confidence-first",
            Self::RiskDriven { .. } => "confidence-first (risk-driven)",
            Self::ThresholdFirst { .. } => "threshold-first",
        }
    }
}

/// Resolves the default baseline directory path from `CARGO_MANIFEST_DIR`.
pub(crate) fn default_baseline_dir() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(manifest_dir).join("tests").join("baselines")
}

/// Builds a spec resolver from `baseline_path` / `baseline_dir` /
/// default.
pub(crate) fn build_default_spec_resolver(
    baseline_path: Option<&Path>,
    baseline_dir: Option<&Path>,
) -> SpecResolver {
    if let Some(path) = baseline_path {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        return SpecResolver::with_dir(parent);
    }
    if let Some(dir) = baseline_dir {
        return SpecResolver::with_dir(dir);
    }
    SpecResolver::with_dir(default_baseline_dir())
}
