//! Threshold resolution from operational approach.
//!
//! Bridges the builder's [`ThresholdApproach`] to the statistics layer's
//! threshold derivation functions, producing the sample count and
//! [`DerivedThreshold`] that drive verdict evaluation.

use crate::ptest::builder::ThresholdApproach;
use crate::statistics::types::{ConfidenceLevel, DerivedThreshold};
use crate::statistics::{defaults, risk_driven_sizing, sample_size, threshold};

/// One baseline-derived criterion's resolved baseline tally.
///
/// Carries the successes and trials the criterion's threshold would be
/// derived from — its own per-criterion measurement where the baseline
/// captured one, otherwise the whole-contract aggregate. Risk-driven
/// resolution sizes each criterion against its tally and lets the largest
/// requirement govern the run.
#[derive(Debug, Clone)]
pub struct CriterionBaselineTally {
    /// The criterion's name, used to attribute the governing requirement.
    pub criterion_name: String,
    /// Baseline successes the criterion derives from.
    pub successes: u32,
    /// Baseline trials the criterion derives from.
    pub trials: u32,
}

impl CriterionBaselineTally {
    /// The tally's observed baseline success rate.
    fn rate(&self) -> f64 {
        f64::from(self.successes) / f64::from(self.trials)
    }
}

/// Resolves the sample count and derived threshold from the approach.
///
/// `criterion_baselines` carries the per-criterion baseline tallies of the
/// contract's baseline-derived criteria; only risk-driven resolution reads
/// it (see [`ThresholdApproach::RiskDriven`]).
///
/// # Panics
///
/// Panics if the approach requires a baseline and none is available, or if
/// a risk-driven plan's `minimum_acceptable_rate` does not sit strictly
/// below the governing baseline rate.
// mavai-ref: JVI-0FVFYBM — do not remove (resolves in mavai-orchestrator)
// mavai-ref: JVI-5YJVXGF — do not remove (resolves in mavai-orchestrator)
// mavai-ref: JVI-6789AKT — do not remove (resolves in mavai-orchestrator)
pub fn resolve_threshold(
    approach: &ThresholdApproach,
    stats: Option<&crate::spec::baseline::StatisticsBlock>,
    execution: Option<&crate::spec::baseline::ExecutionBlock>,
    criterion_baselines: &[CriterionBaselineTally],
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

        ThresholdApproach::RiskDriven {
            minimum_acceptable_rate,
            confidence,
            target_power,
        } => {
            let conf = ConfidenceLevel::new(*confidence);
            let (baseline_successes, baseline_samples) = extract_baseline(stats, execution);
            let aggregate = CriterionBaselineTally {
                criterion_name: "contract aggregate".to_owned(),
                successes: baseline_successes,
                trials: baseline_samples,
            };
            let samples = governing_sample_size(
                *minimum_acceptable_rate,
                conf,
                *target_power,
                criterion_baselines,
                &aggregate,
            );
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

/// The governing sample count for a risk-driven plan: the maximum of the
/// per-criterion required sample sizes, each computed against that
/// criterion's own baseline tally. When the contract carries no
/// baseline-derived criteria, the contract aggregate is sized instead.
///
/// # Panics
///
/// Panics when `minimum_acceptable_rate` does not sit strictly below the
/// governing (lowest) baseline rate — the criterion closest to the declared
/// tolerance is named, since it is the one the sizing cannot satisfy.
fn governing_sample_size(
    minimum_acceptable_rate: f64,
    confidence: ConfidenceLevel,
    target_power: f64,
    criterion_baselines: &[CriterionBaselineTally],
    aggregate: &CriterionBaselineTally,
) -> u32 {
    let tallies: &[CriterionBaselineTally] = if criterion_baselines.is_empty() {
        std::slice::from_ref(aggregate)
    } else {
        criterion_baselines
    };

    let governing = tallies
        .iter()
        .min_by(|a, b| a.rate().total_cmp(&b.rate()))
        .expect("tallies is non-empty by construction");
    assert!(
        minimum_acceptable_rate < governing.rate(),
        "risk-driven sizing is undefined for criterion '{}': \
         minimum_acceptable_rate ({minimum_acceptable_rate}) must sit strictly below \
         the criterion's baseline rate ({}); the tolerance declares how far below the \
         measured baseline a true rate may drop, so to demand more than the baseline \
         delivered, re-measure the baseline rather than raising the tolerance",
        governing.criterion_name,
        governing.rate(),
    );

    tallies
        .iter()
        .map(|tally| {
            risk_driven_sizing::required_sample_size(
                tally.rate(),
                minimum_acceptable_rate,
                confidence,
                target_power,
            )
        })
        .max()
        .expect("tallies is non-empty by construction")
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
/// For `SampleSizeFirst`, `ConfidenceFirst`, and `RiskDriven`, returns the
/// user-supplied confidence. For `ThresholdFirst`, returns the framework
/// default (the confidence-first resolution).
// mavai-ref: JVI-2FYNHXX — do not remove (resolves in mavai-orchestrator)
pub fn resolved_confidence(approach: &ThresholdApproach) -> ConfidenceLevel {
    match approach {
        ThresholdApproach::SampleSizeFirst { confidence, .. }
        | ThresholdApproach::ConfidenceFirst { confidence, .. }
        | ThresholdApproach::RiskDriven { confidence, .. } => ConfidenceLevel::new(*confidence),
        ThresholdApproach::ThresholdFirst { .. } => {
            ConfidenceLevel::new(defaults::DEFAULT_CONFIDENCE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(name: &str, successes: u32, trials: u32) -> CriterionBaselineTally {
        CriterionBaselineTally {
            criterion_name: name.to_owned(),
            successes,
            trials,
        }
    }

    fn aggregate(successes: u32, trials: u32) -> CriterionBaselineTally {
        tally("contract aggregate", successes, trials)
    }

    #[test]
    fn governing_sample_size_takes_the_maximum_over_criteria() {
        let cl = ConfidenceLevel::new(0.95);
        // The lower-rate criterion sits closer to the tolerance and demands
        // more samples; its requirement must govern.
        let strong = tally("format valid", 96, 100);
        let weak = tally("content faithful", 94, 100);
        let weak_alone = governing_sample_size(
            0.93,
            cl,
            0.80,
            std::slice::from_ref(&weak),
            &aggregate(95, 100),
        );
        let strong_alone = governing_sample_size(
            0.93,
            cl,
            0.80,
            std::slice::from_ref(&strong),
            &aggregate(95, 100),
        );
        let both = governing_sample_size(0.93, cl, 0.80, &[strong, weak], &aggregate(95, 100));
        assert!(weak_alone > strong_alone);
        assert_eq!(both, weak_alone);
    }

    #[test]
    fn governing_sample_size_falls_back_to_the_contract_aggregate() {
        let cl = ConfidenceLevel::new(0.95);
        let from_aggregate = governing_sample_size(0.93, cl, 0.80, &[], &aggregate(96, 100));
        let from_criterion = governing_sample_size(
            0.93,
            cl,
            0.80,
            &[tally("only", 96, 100)],
            &aggregate(50, 100),
        );
        assert_eq!(from_aggregate, from_criterion);
    }

    #[test]
    #[should_panic(expected = "undefined for criterion 'content faithful'")]
    fn governing_sample_size_over_reach_names_the_governing_criterion() {
        let cl = ConfidenceLevel::new(0.95);
        governing_sample_size(
            0.95,
            cl,
            0.80,
            &[
                tally("format valid", 98, 100),
                tally("content faithful", 94, 100),
            ],
            &aggregate(96, 100),
        );
    }

    #[test]
    #[should_panic(expected = "undefined for criterion 'contract aggregate'")]
    fn governing_sample_size_over_reach_on_the_aggregate_fallback() {
        let cl = ConfidenceLevel::new(0.95);
        governing_sample_size(0.97, cl, 0.80, &[], &aggregate(96, 100));
    }
}
