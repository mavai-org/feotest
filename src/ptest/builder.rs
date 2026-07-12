//! The threshold-derivation approach and the baseline-resolver helpers shared
//! by the contract-driven probabilistic test.
//!
//! [`ThresholdApproach`] names the four ways a success-rate threshold and a
//! sample count relate; the resolver helpers locate a baseline spec on disk.

use std::path::{Path, PathBuf};

use crate::spec::SpecResolver;

/// Configures the threshold derivation approach.
///
/// Exactly one approach applies to a test. Sample size, confidence, and
/// threshold are mathematically linked: the caller fixes some, the framework
/// derives the rest.
#[derive(Debug, Clone)]
// javai-ref: JVI-0FVFYBM — do not remove (resolves in javai-orchestrator)
// javai-ref: JVI-5YJVXGF — do not remove (resolves in javai-orchestrator)
// javai-ref: JVI-6789AKT — do not remove (resolves in javai-orchestrator)
pub enum ThresholdApproach {
    /// Fix samples and confidence; derive threshold from baseline spec.
    ///
    /// The threshold is the Wilson lower bound at the given confidence
    /// level.
    SampleSizeFirst {
        /// Number of test samples.
        samples: u32,
        /// Confidence level for threshold derivation.
        confidence: f64,
    },

    /// Fix confidence, effect size, and power; derive required sample
    /// count.
    ///
    /// The framework computes the minimum sample size needed to detect
    /// a degradation of `min_detectable_effect` with the given power,
    /// using the fixed-threshold closed form. For a baseline-derived
    /// threshold that closed form is a *seed* — the acceptance floor
    /// moves with the sample size, and the count computed here
    /// understates the requirement. Prefer
    /// [`RiskDriven`](Self::RiskDriven), the same approach priced
    /// self-consistently, when the threshold comes from a measured
    /// baseline.
    ConfidenceFirst {
        /// Required confidence level.
        confidence: f64,
        /// Smallest degradation worth detecting (absolute drop in pass
        /// rate).
        min_detectable_effect: f64,
        /// Probability of detecting a real degradation.
        power: f64,
    },

    /// Fix samples and an explicit threshold; framework derives implied
    /// confidence.
    ThresholdFirst {
        /// Number of test samples.
        samples: u32,
        /// Explicit minimum pass rate.
        min_pass_rate: f64,
    },

    /// Declare a risk appetite; derive the sample count and the threshold.
    ///
    /// This is the **confidence-first** operational approach — the same
    /// approach [`ConfidenceFirst`](Self::ConfidenceFirst) expresses — in
    /// its risk-driven form: the tolerated degradation is stated as an
    /// absolute worst acceptable rate rather than a relative effect size,
    /// and the sizing is priced *self-consistently* against the acceptance
    /// floor the test will actually apply at its own size, rather than by
    /// the fixed-threshold closed form. It is not a different approach;
    /// it fixes the same parameters (confidence, target power, a tolerated
    /// degradation) and derives the sample count from them. Prefer this
    /// form when the threshold comes from a measured baseline: the closed
    /// form understates the sample count there, because the acceptance
    /// floor falls as the sample count shrinks.
    ///
    /// The caller states the worst true success rate they are willing to
    /// tolerate, how confident the test must be, and how often a genuine
    /// breach of that tolerance must be caught. The framework computes the
    /// smallest sample count meeting that promise against the resolved
    /// baseline, and then proceeds exactly as
    /// [`SampleSizeFirst`](Self::SampleSizeFirst) does at that count.
    ///
    /// With several baseline-derived criteria, each criterion is sized
    /// against its own baseline rate and the largest requirement governs
    /// the run.
    ///
    /// Resolving this approach panics if no baseline is available, or if
    /// `minimum_acceptable_rate` does not sit strictly below the governing
    /// baseline rate — the tolerance declares how far below the measured
    /// baseline a true rate may drop, so to demand more than the baseline
    /// delivered, re-measure the baseline rather than raising the tolerance.
    RiskDriven {
        /// The worst true success rate the caller tolerates — a declared
        /// bound, not a measured estimate. Must sit strictly below the
        /// baseline rate.
        minimum_acceptable_rate: f64,
        /// Confidence level for threshold derivation.
        confidence: f64,
        /// Probability that a service truly at the minimum acceptable rate
        /// fails the test (0.80 is a conventional choice).
        target_power: f64,
    },
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
