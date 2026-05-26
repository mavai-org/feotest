//! The threshold-derivation approach and the baseline-resolver helpers shared
//! by the contract-driven probabilistic test.
//!
//! [`ThresholdApproach`] names the three ways a success-rate threshold and a
//! sample count relate; the resolver helpers locate a baseline spec on disk.

use std::path::{Path, PathBuf};

use crate::spec::SpecResolver;

/// Configures the threshold derivation approach.
///
/// Exactly one approach applies to a test. Sample size, confidence, and
/// threshold are mathematically linked: the caller fixes two, the framework
/// derives the third.
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
    /// a degradation of `min_detectable_effect` with the given power.
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
