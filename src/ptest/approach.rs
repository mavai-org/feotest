//! The sampling plan from the operational approach.
//!
//! Bridges the builder's [`ThresholdApproach`] to the statistics layer: the
//! sample count (declared, or sized by resolved sizing against the observed
//! baseline), the test's confidence, the run-level early-termination floor,
//! and what the report discloses about the design.

use crate::ptest::builder::ThresholdApproach;
use crate::statistics::regression::{ImpliedAlpha, fisher_cutoff, implied_alpha};
use crate::statistics::risk_driven_sizing::{self, SizingRefusal};
use crate::statistics::types::ConfidenceLevel;
use crate::statistics::{compliance, defaults};

/// One baseline-derived criterion's resolved baseline tally.
///
/// Carries the successes and trials the criterion's cutoff is derived from —
/// its own per-criterion measurement where the baseline captured one,
/// otherwise the whole-contract aggregate. Resolved sizing sizes each
/// criterion against its tally and lets the largest requirement govern the
/// run.
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
    pub(crate) fn rate(&self) -> f64 {
        f64::from(self.successes) / f64::from(self.trials)
    }
}

/// The true rate at which a sized test is to reach its target power.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DesignAlternative {
    /// A rate stated absolutely (risk-driven).
    Absolute(f64),
    /// A drop below each criterion's own baseline rate (confidence-first).
    BelowBaseline(f64),
}

impl DesignAlternative {
    /// The design alternative rate against a baseline of the given rate.
    #[must_use]
    pub fn rate_for(self, baseline_rate: f64) -> f64 {
        match self {
            Self::Absolute(rate) => rate,
            Self::BelowBaseline(drop) => (baseline_rate - drop).max(0.0),
        }
    }
}

/// The design a sized plan was sized for: the alternative and the power.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizingDesign {
    /// The design alternative rate.
    pub alternative: DesignAlternative,
    /// The target power at it.
    pub target_power: f64,
}

/// A resolved sampling plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunPlan {
    /// The planned sample count.
    pub samples: u32,
    /// The test's confidence: every criterion without its own is decided at
    /// it.
    pub confidence: ConfidenceLevel,
    /// The early-termination floor as a rate: the aggregate regression
    /// cutoff over the sample count, or the declared minimum pass rate;
    /// `0.0` disables early termination.
    pub floor: f64,
    /// The design the plan was sized for, when it was sized.
    pub design: Option<SizingDesign>,
    /// The threshold-first inversion of the declared floor against the
    /// baseline, when one resolved.
    pub implied_alpha: Option<ImpliedAlpha>,
}

impl RunPlan {
    /// The smallest sample count before a guaranteed success may stop the
    /// run early: the count at which the floor could be demonstrated as a
    /// requirement at the test's confidence, so early success never stops a
    /// run that could not yet carry a verdict.
    #[must_use]
    pub fn validity_floor(&self) -> u32 {
        if self.floor > 0.0 && self.floor < 1.0 {
            compliance::minimum_feasible_samples(self.floor, self.confidence.alpha())
        } else {
            1
        }
    }
}

/// Resolves the sampling plan from the approach.
///
/// `criterion_baselines` carries the per-criterion baseline tallies of the
/// contract's baseline-derived criteria; `aggregate` is the baseline's
/// whole-contract tally, when a baseline resolved.
///
/// # Panics
///
/// Panics if the approach needs a baseline and none is available, or if a
/// sized plan's design cannot be priced (see [`ThresholdApproach::RiskDriven`]).
// mavai-ref: JVI-0FVFYBM — do not remove (resolves in mavai-orchestrator)
// mavai-ref: JVI-5YJVXGF — do not remove (resolves in mavai-orchestrator)
// mavai-ref: JVI-6789AKT — do not remove (resolves in mavai-orchestrator)
pub fn resolve_plan(
    approach: &ThresholdApproach,
    aggregate: Option<&CriterionBaselineTally>,
    criterion_baselines: &[CriterionBaselineTally],
) -> RunPlan {
    let confidence = resolved_confidence(approach);
    match approach {
        ThresholdApproach::SampleSizeFirst { samples, .. } => {
            let aggregate = require_baseline(aggregate);
            plan_at(*samples, confidence, aggregate, None)
        }
        ThresholdApproach::ConfidenceFirst {
            min_detectable_effect,
            power,
            ..
        } => {
            let design = SizingDesign {
                alternative: DesignAlternative::BelowBaseline(*min_detectable_effect),
                target_power: *power,
            };
            sized_plan(design, confidence, aggregate, criterion_baselines)
        }
        ThresholdApproach::RiskDriven {
            design_alternative_rate,
            target_power,
            ..
        } => {
            let design = SizingDesign {
                alternative: DesignAlternative::Absolute(*design_alternative_rate),
                target_power: *target_power,
            };
            sized_plan(design, confidence, aggregate, criterion_baselines)
        }
        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        } => RunPlan {
            samples: *samples,
            confidence,
            floor: *min_pass_rate,
            design: None,
            implied_alpha: aggregate
                .filter(|tally| tally.trials > 0)
                .map(|tally| threshold_first_inversion(tally, *samples, *min_pass_rate)),
        },
    }
}

/// A plan at a fixed size, its floor the aggregate regression cutoff.
fn plan_at(
    samples: u32,
    confidence: ConfidenceLevel,
    aggregate: &CriterionBaselineTally,
    design: Option<SizingDesign>,
) -> RunPlan {
    RunPlan {
        samples,
        confidence,
        floor: aggregate_floor(aggregate, samples, confidence),
        design,
        implied_alpha: None,
    }
}

/// The resolved-sizing plan: each criterion (or the aggregate) sized against
/// its own tally, the largest requirement governing.
fn sized_plan(
    design: SizingDesign,
    confidence: ConfidenceLevel,
    aggregate: Option<&CriterionBaselineTally>,
    criterion_baselines: &[CriterionBaselineTally],
) -> RunPlan {
    let aggregate = require_baseline(aggregate);
    let samples = governing_sample_size(design, confidence, criterion_baselines, aggregate);
    plan_at(samples, confidence, aggregate, Some(design))
}

/// The aggregate regression cutoff over the sample count, as a rate.
fn aggregate_floor(
    aggregate: &CriterionBaselineTally,
    samples: u32,
    confidence: ConfidenceLevel,
) -> f64 {
    if aggregate.trials == 0 || samples == 0 {
        return 0.0;
    }
    let cutoff = fisher_cutoff(
        aggregate.successes,
        aggregate.trials,
        samples,
        confidence.alpha(),
    );
    f64::from(cutoff) / f64::from(samples)
}

/// The implied alpha of the declared floor's cutoff, `⌈rate · n⌉`.
fn threshold_first_inversion(
    aggregate: &CriterionBaselineTally,
    samples: u32,
    min_pass_rate: f64,
) -> ImpliedAlpha {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a rate in [0, 1] times a u32 size is a non-negative count within u32"
    )]
    let cutoff = f64::from(samples)
        .mul_add(min_pass_rate, -1e-9)
        .ceil()
        .max(0.0) as u32;
    implied_alpha(
        aggregate.successes,
        aggregate.trials,
        samples,
        cutoff.min(samples),
    )
}

/// The baseline an approach needs.
///
/// # Panics
///
/// Panics if no baseline resolved.
const fn require_baseline(aggregate: Option<&CriterionBaselineTally>) -> &CriterionBaselineTally {
    aggregate.expect("baseline spec required for this threshold approach")
}

/// The governing sample count for a sized plan: the maximum of the
/// per-criterion resolved-sizing requirements, each computed against that
/// criterion's own baseline tally. When the contract carries no
/// baseline-derived criteria, the contract aggregate is sized instead.
///
/// # Panics
///
/// Panics when any criterion's design cannot be priced, naming the criterion
/// and the refusal.
fn governing_sample_size(
    design: SizingDesign,
    confidence: ConfidenceLevel,
    criterion_baselines: &[CriterionBaselineTally],
    aggregate: &CriterionBaselineTally,
) -> u32 {
    let tallies: &[CriterionBaselineTally] = if criterion_baselines.is_empty() {
        std::slice::from_ref(aggregate)
    } else {
        criterion_baselines
    };
    // A design outside a tally's domain is refused before any sizing runs,
    // whichever tally it concerns.
    for tally in tallies {
        let rate = tally.rate();
        if let Some(refusal) = risk_driven_sizing::check_sizing_domain(
            rate,
            tally.trials,
            Some(design.alternative.rate_for(rate)),
            None,
        ) {
            refuse(tally, refusal, design);
        }
    }
    tallies
        .iter()
        .map(|tally| required_samples(design, confidence, tally))
        .max()
        .expect("tallies is non-empty by construction")
}

/// Panics with a sizing refusal's message, naming the criterion.
fn refuse(tally: &CriterionBaselineTally, refusal: SizingRefusal, design: SizingDesign) -> ! {
    let rate = tally.rate();
    panic!(
        "sizing is undefined for criterion '{}' ({}): {}",
        tally.criterion_name,
        refusal.category(),
        refusal.message(rate, design.alternative.rate_for(rate))
    )
}

/// One tally's resolved-sizing requirement; the tally's domain has been
/// checked.
///
/// # Panics
///
/// Panics, naming the criterion, when no test up to the baseline's size
/// reaches and holds the target power.
fn required_samples(
    design: SizingDesign,
    confidence: ConfidenceLevel,
    tally: &CriterionBaselineTally,
) -> u32 {
    risk_driven_sizing::resolved_sizing(
        tally.successes,
        tally.trials,
        design.alternative.rate_for(tally.rate()),
        confidence.alpha(),
        design.target_power,
    )
    .unwrap_or_else(|| refuse(tally, SizingRefusal::BaselineTooSmall, design))
    .required_samples()
}

/// Extracts the resolved confidence level from an approach.
///
/// For `SampleSizeFirst`, `ConfidenceFirst`, and `RiskDriven`, returns the
/// user-supplied confidence. For `ThresholdFirst`, returns the framework
/// default.
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

    fn design(rate: f64) -> SizingDesign {
        SizingDesign {
            alternative: DesignAlternative::Absolute(rate),
            target_power: 0.80,
        }
    }

    fn cl() -> ConfidenceLevel {
        ConfidenceLevel::new(0.95)
    }

    #[test]
    fn governing_sample_size_takes_the_maximum_over_criteria() {
        // The lower-rate criterion sits closer to the design alternative and
        // demands more samples; its requirement must govern.
        let strong = tally("format valid", 1920, 2000);
        let weak = tally("content faithful", 1900, 2000);
        let aggregate = tally("contract aggregate", 1910, 2000);
        let weak_alone =
            governing_sample_size(design(0.90), cl(), std::slice::from_ref(&weak), &aggregate);
        let strong_alone = governing_sample_size(
            design(0.90),
            cl(),
            std::slice::from_ref(&strong),
            &aggregate,
        );
        let both = governing_sample_size(design(0.90), cl(), &[strong, weak], &aggregate);
        assert!(weak_alone > strong_alone);
        assert_eq!(both, weak_alone);
    }

    #[test]
    fn governing_sample_size_falls_back_to_the_contract_aggregate() {
        let from_aggregate =
            governing_sample_size(design(0.93), cl(), &[], &tally("aggregate", 1920, 2000));
        let from_criterion = governing_sample_size(
            design(0.93),
            cl(),
            &[tally("only", 1920, 2000)],
            &tally("aggregate", 1000, 2000),
        );
        assert_eq!(from_aggregate, from_criterion);
    }

    #[test]
    #[should_panic(
        expected = "undefined for criterion 'content faithful' (ALTERNATIVE_NOT_BELOW_BASELINE)"
    )]
    fn an_alternative_above_a_criterion_baseline_names_the_criterion() {
        governing_sample_size(
            design(0.95),
            cl(),
            &[
                tally("format valid", 98, 100),
                tally("content faithful", 94, 100),
            ],
            &tally("aggregate", 96, 100),
        );
    }

    #[test]
    #[should_panic(expected = "(BASELINE_TOO_SMALL)")]
    fn a_baseline_too_small_for_the_design_is_refused() {
        governing_sample_size(design(0.93), cl(), &[], &tally("aggregate", 288, 300));
    }

    #[test]
    fn confidence_first_sizes_below_each_baseline() {
        let alternative = DesignAlternative::BelowBaseline(0.03);
        assert!((alternative.rate_for(0.96) - 0.93).abs() < 1e-12);
        assert!(alternative.rate_for(0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn threshold_first_discloses_the_implied_alpha_of_its_cutoff() {
        let plan = resolve_plan(
            &ThresholdApproach::ThresholdFirst {
                samples: 100,
                min_pass_rate: 0.91,
            },
            Some(&tally("aggregate", 951, 1000)),
            &[],
        );
        let implied = plan.implied_alpha.unwrap();
        assert!(implied.alpha().unwrap() <= 0.05);
        assert_eq!(implied.is_sound(), Some(true));
    }

    #[test]
    fn sample_size_first_floors_at_the_aggregate_cutoff() {
        let plan = resolve_plan(
            &ThresholdApproach::SampleSizeFirst {
                samples: 100,
                confidence: 0.95,
            },
            Some(&tally("aggregate", 951, 1000)),
            &[],
        );
        assert!((plan.floor - 0.91).abs() < 1e-12);
        assert_eq!(plan.samples, 100);
    }

    #[test]
    fn validity_floor_is_the_feasibility_minimum_of_the_floor() {
        let plan = RunPlan {
            samples: 100,
            confidence: cl(),
            floor: 0.95,
            design: None,
            implied_alpha: None,
        };
        assert_eq!(plan.validity_floor(), 59);
        let bare = RunPlan { floor: 0.0, ..plan };
        assert_eq!(bare.validity_floor(), 1);
    }
}
