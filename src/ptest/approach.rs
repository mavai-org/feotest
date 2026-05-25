//! Threshold resolution from operational approach.
//!
//! Bridges the builder's [`ThresholdApproach`] to the statistics layer's
//! threshold derivation functions, producing the sample count and
//! [`DerivedThreshold`] that drive verdict evaluation.

use crate::ptest::builder::ThresholdApproach;
use crate::statistics::types::{ConfidenceLevel, DerivedThreshold};
use crate::statistics::{defaults, sample_size, threshold};

/// Resolves the sample count and derived threshold from the approach.
// javai-ref: JVI-0FVFYBM — do not remove (resolves in javai-orchestrator)
// javai-ref: JVI-5YJVXGF — do not remove (resolves in javai-orchestrator)
// javai-ref: JVI-6789AKT — do not remove (resolves in javai-orchestrator)
pub fn resolve_threshold(
    approach: &ThresholdApproach,
    stats: Option<&crate::spec::baseline::StatisticsBlock>,
    execution: Option<&crate::spec::baseline::ExecutionBlock>,
) -> (u32, DerivedThreshold) {
    match approach {
        ThresholdApproach::SampleSizeFirst {
            samples,
            confidence,
        } => {
            let conf = ConfidenceLevel::new(*confidence);
            let (baseline_successes, baseline_samples) = extract_baseline(stats, execution);
            let derived = threshold::derive_sample_size_first(
                baseline_successes,
                baseline_samples,
                *samples,
                conf,
            );
            (*samples, derived)
        }

        ThresholdApproach::ConfidenceFirst {
            confidence,
            min_detectable_effect,
            power,
        } => {
            let conf = ConfidenceLevel::new(*confidence);
            let (baseline_successes, baseline_samples) = extract_baseline(stats, execution);
            let baseline_rate = f64::from(baseline_successes) / f64::from(baseline_samples);

            let requirement = sample_size::calculate_for_power(
                baseline_rate,
                *min_detectable_effect,
                conf,
                *power,
            );

            let samples = requirement.required_samples();
            let derived = threshold::derive_sample_size_first(
                baseline_successes,
                baseline_samples,
                samples,
                conf,
            );
            (samples, derived)
        }

        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        } => {
            if let (Some(s), Some(e)) = (stats, execution) {
                let baseline_successes = s.successes;
                let baseline_samples = e.samples_executed;
                let derived = threshold::derive_threshold_first(
                    baseline_successes,
                    baseline_samples,
                    *samples,
                    *min_pass_rate,
                );
                (*samples, derived)
            } else {
                let conf = ConfidenceLevel::new(defaults::DEFAULT_CONFIDENCE);
                let context = crate::statistics::types::DerivationContext::new(
                    *min_pass_rate,
                    *samples,
                    *samples,
                    conf,
                );
                let derived = DerivedThreshold::new(
                    *min_pass_rate,
                    crate::statistics::types::OperationalApproach::ThresholdFirst,
                    context,
                    false,
                );
                (*samples, derived)
            }
        }
    }
}

/// Extracts baseline successes and sample count from spec blocks.
///
/// # Panics
///
/// Panics if no baseline data is available (spec is required for
/// sample-size-first and confidence-first approaches).
const fn extract_baseline(
    stats: Option<&crate::spec::baseline::StatisticsBlock>,
    execution: Option<&crate::spec::baseline::ExecutionBlock>,
) -> (u32, u32) {
    let stats = stats.expect("baseline spec required for this threshold approach");
    let execution = execution.expect("baseline spec required for this threshold approach");
    (stats.successes, execution.samples_executed)
}

/// Extracts the resolved confidence level from an approach.
///
/// For `SampleSizeFirst` and `ConfidenceFirst`, returns the user-supplied
/// confidence. For `ThresholdFirst`, returns the framework default (the
/// confidence-first resolution).
// javai-ref: JVI-2FYNHXX — do not remove (resolves in javai-orchestrator)
pub fn resolved_confidence(approach: &ThresholdApproach) -> ConfidenceLevel {
    match approach {
        ThresholdApproach::SampleSizeFirst { confidence, .. }
        | ThresholdApproach::ConfidenceFirst { confidence, .. } => {
            ConfidenceLevel::new(*confidence)
        }
        ThresholdApproach::ThresholdFirst { .. } => {
            ConfidenceLevel::new(defaults::DEFAULT_CONFIDENCE)
        }
    }
}
