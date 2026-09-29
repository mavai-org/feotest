//! Sizing-transparency facts recorded with the verdict.
//!
//! A report shows verdicts and statistics, but is silent about the deal the
//! operator struck when sizing the run: which operational approach shaped
//! the design, what a smaller-than-baseline sample count cost in
//! sensitivity, and what it saved in time and tokens. This module computes
//! those facts in the engine layer — the sensitivity figure through the
//! same sizing statistics the run itself uses — and records them on the
//! verdict record as free-form environment entries, so the verdict schema
//! is unchanged and every renderer formats already-computed values.

use crate::model::ExecutionSummary;
use crate::ptest::approach::{CriterionBaselineTally, RunPlan};
use crate::ptest::builder::ThresholdApproach;
use crate::statistics::defaults::DEFAULT_TARGET_POWER;
use crate::statistics::regression;

/// The operational approach's canonical name.
const APPROACH_KEY: &str = "sizing-approach";
/// Sample count the operator declared (sample-size-first, threshold-first).
const DECLARED_SAMPLES_KEY: &str = "sizing-declared-samples";
/// Confidence the operator declared.
const DECLARED_CONFIDENCE_KEY: &str = "sizing-declared-confidence";
/// Target power the operator declared.
const DECLARED_POWER_KEY: &str = "sizing-declared-power";
/// Smallest degradation worth detecting (confidence-first).
const DECLARED_EFFECT_KEY: &str = "sizing-declared-min-detectable-effect";
/// Explicit minimum pass rate the operator declared (threshold-first).
const DECLARED_MIN_PASS_RATE_KEY: &str = "sizing-declared-min-pass-rate";
/// The true rate at which the test is to reach its target power
/// (risk-driven).
const DESIGN_ALTERNATIVE_RATE_KEY: &str = "sizing-design-alternative-rate";
/// Sample count the framework computed from the declared parameters.
const COMPUTED_SAMPLES_KEY: &str = "sizing-computed-samples";
/// The implied alpha of a declared threshold-first cutoff.
const IMPLIED_ALPHA_KEY: &str = "sizing-implied-alpha";
/// Whether the implied alpha is sound (at most 0.20).
const IMPLIED_ALPHA_SOUND_KEY: &str = "sizing-implied-alpha-sound";
/// Largest true rate the resolved test detects at the stated power.
const DETECTABLE_RATE_KEY: &str = "sizing-detectable-rate";
/// The power at which the detectable rate is stated.
const DETECTABLE_POWER_KEY: &str = "sizing-detectable-power";
/// Fraction of a baseline-sized run's cost the smaller run saves.
const SAVED_FRACTION_KEY: &str = "sizing-saved-fraction";
/// Estimated execution time saved, in milliseconds.
const TIME_SAVED_MS_KEY: &str = "sizing-time-saved-ms";
/// Estimated tokens saved (absent when the run recorded no token costs).
const TOKENS_SAVED_KEY: &str = "sizing-tokens-saved";

/// Computes the sizing-transparency entries one verdict record carries.
///
/// The approach entry (with its declared parameters) is always present; a
/// threshold-first plan against a baseline adds the implied alpha of its
/// cutoff. The downsizing pair — the detectable rate at the run's size and
/// the estimated savings versus a baseline-sized run — is present iff the
/// run was sized below the resolved baseline's own sampling size; the token
/// half of the savings is present iff the run recorded token costs.
///
/// The detectable rate is the largest true rate the test, resolved against
/// the observed baseline under `regression/fisher`, detects at the stated
/// power: against the weakest (lowest-rate) baseline-derived criterion's
/// tally when the contract carries any, otherwise the baseline's
/// whole-contract tally.
// mavai-ref: JVI-RX30FM8 — do not remove (resolves in mavai-orchestrator)
pub(super) fn sizing_disclosure_entries(
    approach: &ThresholdApproach,
    plan: &RunPlan,
    aggregate: Option<&CriterionBaselineTally>,
    criterion_tallies: &[CriterionBaselineTally],
    execution: &ExecutionSummary,
) -> Vec<(String, String)> {
    let mut entries = approach_entries(approach, execution.samples_planned());
    if let Some(implied) = plan.implied_alpha
        && let (Some(alpha), Some(sound)) = (implied.alpha(), implied.is_sound())
    {
        push(&mut entries, IMPLIED_ALPHA_KEY, alpha.to_string());
        push(&mut entries, IMPLIED_ALPHA_SOUND_KEY, sound.to_string());
    }
    if let Some(aggregate) = aggregate {
        let governing = governing_tally(criterion_tallies, aggregate);
        downsizing_entries(&mut entries, approach, plan, governing, execution);
    }
    entries
}

/// Appends one entry.
fn push(entries: &mut Vec<(String, String)>, key: &str, value: String) {
    entries.push((key.to_owned(), value));
}

/// The approach's name and its declared parameters.
fn approach_entries(approach: &ThresholdApproach, planned: u32) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    push(
        &mut entries,
        APPROACH_KEY,
        approach.canonical_name().to_owned(),
    );
    match approach {
        ThresholdApproach::SampleSizeFirst {
            samples,
            confidence,
        } => {
            push(&mut entries, DECLARED_SAMPLES_KEY, samples.to_string());
            push(
                &mut entries,
                DECLARED_CONFIDENCE_KEY,
                confidence.to_string(),
            );
        }
        ThresholdApproach::ConfidenceFirst {
            confidence,
            min_detectable_effect,
            power,
        } => {
            push(
                &mut entries,
                DECLARED_CONFIDENCE_KEY,
                confidence.to_string(),
            );
            push(
                &mut entries,
                DECLARED_EFFECT_KEY,
                min_detectable_effect.to_string(),
            );
            push(&mut entries, DECLARED_POWER_KEY, power.to_string());
            push(&mut entries, COMPUTED_SAMPLES_KEY, planned.to_string());
        }
        ThresholdApproach::RiskDriven {
            design_alternative_rate,
            confidence,
            target_power,
        } => {
            push(
                &mut entries,
                DESIGN_ALTERNATIVE_RATE_KEY,
                design_alternative_rate.to_string(),
            );
            push(
                &mut entries,
                DECLARED_CONFIDENCE_KEY,
                confidence.to_string(),
            );
            push(&mut entries, DECLARED_POWER_KEY, target_power.to_string());
            push(&mut entries, COMPUTED_SAMPLES_KEY, planned.to_string());
        }
        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        } => {
            push(&mut entries, DECLARED_SAMPLES_KEY, samples.to_string());
            push(
                &mut entries,
                DECLARED_MIN_PASS_RATE_KEY,
                min_pass_rate.to_string(),
            );
        }
    }
    entries
}

/// The downsizing pair, when the run was sized below the baseline's size and
/// the governing tally has a rate strictly between 0 and 1.
fn downsizing_entries(
    entries: &mut Vec<(String, String)>,
    approach: &ThresholdApproach,
    plan: &RunPlan,
    governing: &CriterionBaselineTally,
    execution: &ExecutionSummary,
) {
    let planned = execution.samples_planned();
    let downsized = planned > 0
        && planned < governing.trials
        && governing.successes > 0
        && governing.successes < governing.trials;
    if !downsized {
        return;
    }
    let power = match approach {
        ThresholdApproach::RiskDriven { target_power, .. } => *target_power,
        ThresholdApproach::ConfidenceFirst { power, .. } => *power,
        _ => DEFAULT_TARGET_POWER,
    };
    if let Some(detectable) = regression::resolved_detectable_rate(
        governing.successes,
        governing.trials,
        planned,
        plan.confidence.alpha(),
        power,
    ) {
        push(entries, DETECTABLE_RATE_KEY, detectable.to_string());
        push(entries, DETECTABLE_POWER_KEY, power.to_string());
    }
    let saved_samples = governing.trials - planned;
    let fraction = f64::from(saved_samples) / f64::from(governing.trials);
    push(entries, SAVED_FRACTION_KEY, fraction.to_string());
    let time_saved_ms =
        execution.cost().avg_time_per_sample().as_millis() * u128::from(saved_samples);
    push(entries, TIME_SAVED_MS_KEY, time_saved_ms.to_string());
    if execution.cost().total_tokens() > 0 {
        let tokens_saved = execution.cost().avg_tokens_per_sample() * u64::from(saved_samples);
        push(entries, TOKENS_SAVED_KEY, tokens_saved.to_string());
    }
}

/// The tally the run's sizing runs against: the weakest (lowest-rate)
/// baseline-derived criterion tally when any exist, else the baseline's
/// whole-contract tally.
fn governing_tally<'a>(
    tallies: &'a [CriterionBaselineTally],
    aggregate: &'a CriterionBaselineTally,
) -> &'a CriterionBaselineTally {
    tallies
        .iter()
        .min_by(|a, b| a.rate().total_cmp(&b.rate()))
        .unwrap_or(aggregate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CostSummary, TerminationInfo, TerminationReason};
    use crate::statistics::types::ConfidenceLevel;
    use std::time::Duration;

    fn execution(planned: u32, total_ms: u64, tokens: u64) -> ExecutionSummary {
        ExecutionSummary::new(
            planned,
            planned,
            planned,
            0,
            TerminationInfo::new(TerminationReason::Completed),
            CostSummary::new(Duration::from_millis(total_ms), tokens, planned),
        )
    }

    fn tally(name: &str, successes: u32, trials: u32) -> CriterionBaselineTally {
        CriterionBaselineTally {
            criterion_name: name.to_owned(),
            successes,
            trials,
        }
    }

    fn plan(samples: u32) -> RunPlan {
        RunPlan {
            samples,
            confidence: ConfidenceLevel::new(0.95),
            floor: 0.0,
            design: None,
            implied_alpha: None,
        }
    }

    fn sample_size_first() -> ThresholdApproach {
        ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        }
    }

    fn value<'a>(entries: &'a [(String, String)], key: &str) -> Option<&'a str> {
        entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn every_run_discloses_its_approach() {
        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.9,
        };
        let entries =
            sizing_disclosure_entries(&approach, &plan(100), None, &[], &execution(100, 5000, 0));
        assert_eq!(value(&entries, APPROACH_KEY), Some("threshold-first"));
        assert_eq!(value(&entries, DECLARED_SAMPLES_KEY), Some("100"));
        assert_eq!(value(&entries, DECLARED_MIN_PASS_RATE_KEY), Some("0.9"));
    }

    #[test]
    fn risk_driven_disclosure_names_the_design_alternative_rate() {
        let approach = ThresholdApproach::RiskDriven {
            design_alternative_rate: 0.85,
            confidence: 0.95,
            target_power: 0.8,
        };
        let entries =
            sizing_disclosure_entries(&approach, &plan(180), None, &[], &execution(180, 9000, 0));
        assert_eq!(
            value(&entries, APPROACH_KEY),
            Some("confidence-first (risk-driven)")
        );
        assert_eq!(value(&entries, DESIGN_ALTERNATIVE_RATE_KEY), Some("0.85"));
        assert_eq!(value(&entries, DECLARED_CONFIDENCE_KEY), Some("0.95"));
        assert_eq!(value(&entries, DECLARED_POWER_KEY), Some("0.8"));
        assert_eq!(value(&entries, COMPUTED_SAMPLES_KEY), Some("180"));
    }

    #[test]
    fn threshold_first_discloses_its_implied_alpha() {
        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.91,
        };
        let mut with_inversion = plan(100);
        with_inversion.implied_alpha = Some(regression::implied_alpha(951, 1000, 100, 91));
        let entries = sizing_disclosure_entries(
            &approach,
            &with_inversion,
            None,
            &[],
            &execution(100, 5000, 0),
        );
        assert!(value(&entries, IMPLIED_ALPHA_KEY).is_some());
        assert_eq!(value(&entries, IMPLIED_ALPHA_SOUND_KEY), Some("true"));
    }

    #[test]
    fn downsizing_pair_appears_iff_planned_below_baseline_size() {
        let downsized = sizing_disclosure_entries(
            &sample_size_first(),
            &plan(100),
            Some(&tally("aggregate", 960, 1000)),
            &[],
            &execution(100, 5000, 120_000),
        );
        assert!(value(&downsized, DETECTABLE_RATE_KEY).is_some());
        assert!(value(&downsized, SAVED_FRACTION_KEY).is_some());

        let full_size = sizing_disclosure_entries(
            &sample_size_first(),
            &plan(100),
            Some(&tally("aggregate", 96, 100)),
            &[],
            &execution(100, 5000, 120_000),
        );
        assert!(value(&full_size, DETECTABLE_RATE_KEY).is_none());
        assert!(value(&full_size, SAVED_FRACTION_KEY).is_none());

        let baseline_less = sizing_disclosure_entries(
            &sample_size_first(),
            &plan(100),
            None,
            &[],
            &execution(100, 5000, 120_000),
        );
        assert!(value(&baseline_less, DETECTABLE_RATE_KEY).is_none());
    }

    #[test]
    fn detectable_rate_is_the_resolved_detectable_rate_of_the_weakest_tally() {
        let tallies = vec![
            tally("format valid", 980, 1000),
            tally("content faithful", 940, 1000),
        ];
        let entries = sizing_disclosure_entries(
            &sample_size_first(),
            &plan(100),
            Some(&tally("aggregate", 960, 1000)),
            &tallies,
            &execution(100, 5000, 0),
        );
        let disclosed: f64 = value(&entries, DETECTABLE_RATE_KEY)
            .unwrap()
            .parse()
            .unwrap();
        let expected =
            regression::resolved_detectable_rate(940, 1000, 100, 0.05, DEFAULT_TARGET_POWER)
                .unwrap();
        assert!((disclosed - expected).abs() < 1e-12);
        assert_eq!(value(&entries, DETECTABLE_POWER_KEY), Some("0.8"));
    }

    #[test]
    fn savings_derive_from_the_run_recorded_costs() {
        // 100 samples over 5,000 ms and 120,000 tokens: 50 ms and 1,200
        // tokens per sample; 900 saved samples versus the baseline's 1,000.
        let entries = sizing_disclosure_entries(
            &sample_size_first(),
            &plan(100),
            Some(&tally("aggregate", 960, 1000)),
            &[],
            &execution(100, 5000, 120_000),
        );
        assert_eq!(value(&entries, SAVED_FRACTION_KEY), Some("0.9"));
        assert_eq!(value(&entries, TIME_SAVED_MS_KEY), Some("45000"));
        assert_eq!(value(&entries, TOKENS_SAVED_KEY), Some("1080000"));
    }

    #[test]
    fn token_half_degrades_away_when_no_tokens_are_recorded() {
        let entries = sizing_disclosure_entries(
            &sample_size_first(),
            &plan(100),
            Some(&tally("aggregate", 960, 1000)),
            &[],
            &execution(100, 5000, 0),
        );
        assert!(value(&entries, TIME_SAVED_MS_KEY).is_some());
        assert!(value(&entries, TOKENS_SAVED_KEY).is_none());
    }

    #[test]
    fn a_perfect_baseline_suppresses_the_downsizing_pair() {
        let entries = sizing_disclosure_entries(
            &sample_size_first(),
            &plan(100),
            Some(&tally("aggregate", 1000, 1000)),
            &[],
            &execution(100, 5000, 0),
        );
        assert_eq!(value(&entries, APPROACH_KEY), Some("sample-size-first"));
        assert!(value(&entries, DETECTABLE_RATE_KEY).is_none());
        assert!(value(&entries, SAVED_FRACTION_KEY).is_none());
    }
}
