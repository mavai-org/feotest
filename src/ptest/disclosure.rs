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
use crate::ptest::approach::CriterionBaselineTally;
use crate::ptest::builder::ThresholdApproach;
use crate::statistics::defaults::DEFAULT_TARGET_POWER;
use crate::statistics::risk_driven_sizing;
use crate::statistics::types::ConfidenceLevel;
use crate::verdict::BaselineProvenance;

/// The operational approach's canonical name.
const APPROACH_KEY: &str = "sizing-approach";
/// Sample count the operator declared (sample-size-first, threshold-first).
const DECLARED_SAMPLES_KEY: &str = "sizing-declared-samples";
/// Confidence the operator declared.
const DECLARED_CONFIDENCE_KEY: &str = "sizing-declared-confidence";
/// Target power the operator declared.
const DECLARED_POWER_KEY: &str = "sizing-declared-power";
/// Smallest degradation worth detecting (closed-form confidence-first).
const DECLARED_EFFECT_KEY: &str = "sizing-declared-min-detectable-effect";
/// Explicit minimum pass rate the operator declared (threshold-first).
const DECLARED_MIN_PASS_RATE_KEY: &str = "sizing-declared-min-pass-rate";
/// Worst true rate the operator tolerates (risk-driven).
const TOLERATED_RATE_KEY: &str = "sizing-tolerated-rate";
/// Sample count the framework computed from the declared parameters.
const COMPUTED_SAMPLES_KEY: &str = "sizing-computed-samples";
/// Largest tolerable true rate detectable at the run's size.
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
/// The approach entry (with its declared parameters) is always present.
/// The downsizing pair — the detectable rate at the run's size and the
/// estimated savings versus a baseline-sized run — is present iff the run
/// was sized below the resolved baseline's own sampling size; the token
/// half of the savings is present iff the run recorded token costs.
///
/// The detectable rate is computed against the rate the sizing itself runs
/// against: the weakest (lowest-rate) baseline-derived criterion's tally
/// when the contract carries any, otherwise the baseline's whole-contract
/// rate.
// javai-ref: JVI-RX30FM8 — do not remove (resolves in javai-orchestrator)
pub(super) fn sizing_disclosure_entries(
    approach: &ThresholdApproach,
    confidence: ConfidenceLevel,
    baseline: Option<&BaselineProvenance>,
    criterion_tallies: &[CriterionBaselineTally],
    execution: &ExecutionSummary,
) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let entry = |entries: &mut Vec<(String, String)>, key: &str, value: String| {
        entries.push((key.to_owned(), value));
    };

    entry(
        &mut entries,
        APPROACH_KEY,
        approach.canonical_name().to_owned(),
    );
    let planned = execution.samples_planned();
    match approach {
        ThresholdApproach::SampleSizeFirst {
            samples,
            confidence,
        } => {
            entry(&mut entries, DECLARED_SAMPLES_KEY, samples.to_string());
            entry(
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
            entry(
                &mut entries,
                DECLARED_CONFIDENCE_KEY,
                confidence.to_string(),
            );
            entry(
                &mut entries,
                DECLARED_EFFECT_KEY,
                min_detectable_effect.to_string(),
            );
            entry(&mut entries, DECLARED_POWER_KEY, power.to_string());
            entry(&mut entries, COMPUTED_SAMPLES_KEY, planned.to_string());
        }
        ThresholdApproach::RiskDriven {
            minimum_acceptable_rate,
            confidence,
            target_power,
        } => {
            entry(
                &mut entries,
                TOLERATED_RATE_KEY,
                minimum_acceptable_rate.to_string(),
            );
            entry(
                &mut entries,
                DECLARED_CONFIDENCE_KEY,
                confidence.to_string(),
            );
            entry(&mut entries, DECLARED_POWER_KEY, target_power.to_string());
            entry(&mut entries, COMPUTED_SAMPLES_KEY, planned.to_string());
        }
        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        } => {
            entry(&mut entries, DECLARED_SAMPLES_KEY, samples.to_string());
            entry(
                &mut entries,
                DECLARED_MIN_PASS_RATE_KEY,
                min_pass_rate.to_string(),
            );
        }
    }

    if let Some(baseline) = baseline {
        let sizing_rate = governing_rate(criterion_tallies, baseline);
        let downsized = planned < baseline.baseline_samples()
            && planned > 0
            && sizing_rate > 0.0
            && sizing_rate < 1.0;
        if downsized {
            let power = match approach {
                ThresholdApproach::RiskDriven { target_power, .. } => *target_power,
                _ => DEFAULT_TARGET_POWER,
            };
            let detectable =
                risk_driven_sizing::detectable_rate(planned, sizing_rate, confidence, power);
            entry(&mut entries, DETECTABLE_RATE_KEY, detectable.to_string());
            entry(&mut entries, DETECTABLE_POWER_KEY, power.to_string());

            let saved_samples = baseline.baseline_samples() - planned;
            let fraction = f64::from(saved_samples) / f64::from(baseline.baseline_samples());
            entry(&mut entries, SAVED_FRACTION_KEY, fraction.to_string());
            let time_saved_ms =
                execution.cost().avg_time_per_sample().as_millis() * u128::from(saved_samples);
            entry(&mut entries, TIME_SAVED_MS_KEY, time_saved_ms.to_string());
            if execution.cost().total_tokens() > 0 {
                let tokens_saved =
                    execution.cost().avg_tokens_per_sample() * u64::from(saved_samples);
                entry(&mut entries, TOKENS_SAVED_KEY, tokens_saved.to_string());
            }
        }
    }

    entries
}

/// The rate the run's sizing runs against: the weakest (lowest-rate)
/// baseline-derived criterion tally when any exist, else the baseline's
/// whole-contract observed rate.
fn governing_rate(tallies: &[CriterionBaselineTally], baseline: &BaselineProvenance) -> f64 {
    tallies
        .iter()
        .map(|tally| f64::from(tally.successes) / f64::from(tally.trials))
        .min_by(f64::total_cmp)
        .unwrap_or_else(|| baseline.baseline_rate())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CostSummary, TerminationInfo, TerminationReason};
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

    fn baseline(samples: u32, rate: f64) -> BaselineProvenance {
        BaselineProvenance::new("svc.yaml", "2026-07-01T00:00:00Z", samples, rate, 0.9)
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
        let entries = sizing_disclosure_entries(
            &approach,
            ConfidenceLevel::new(0.95),
            None,
            &[],
            &execution(100, 5000, 0),
        );
        assert_eq!(value(&entries, APPROACH_KEY), Some("threshold-first"));
        assert_eq!(value(&entries, DECLARED_SAMPLES_KEY), Some("100"));
        assert_eq!(value(&entries, DECLARED_MIN_PASS_RATE_KEY), Some("0.9"));
    }

    #[test]
    fn risk_driven_disclosure_names_the_confidence_first_approach() {
        let approach = ThresholdApproach::RiskDriven {
            minimum_acceptable_rate: 0.85,
            confidence: 0.95,
            target_power: 0.8,
        };
        let entries = sizing_disclosure_entries(
            &approach,
            ConfidenceLevel::new(0.95),
            None,
            &[],
            &execution(180, 9000, 0),
        );
        assert_eq!(
            value(&entries, APPROACH_KEY),
            Some("confidence-first (risk-driven)")
        );
        assert_eq!(value(&entries, TOLERATED_RATE_KEY), Some("0.85"));
        assert_eq!(value(&entries, DECLARED_CONFIDENCE_KEY), Some("0.95"));
        assert_eq!(value(&entries, DECLARED_POWER_KEY), Some("0.8"));
        assert_eq!(value(&entries, COMPUTED_SAMPLES_KEY), Some("180"));
    }

    #[test]
    fn downsizing_pair_appears_iff_planned_below_baseline_size() {
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        let confidence = ConfidenceLevel::new(0.95);
        let downsized = sizing_disclosure_entries(
            &approach,
            confidence,
            Some(&baseline(1000, 0.96)),
            &[],
            &execution(100, 5000, 120_000),
        );
        assert!(value(&downsized, DETECTABLE_RATE_KEY).is_some());
        assert!(value(&downsized, SAVED_FRACTION_KEY).is_some());

        let full_size = sizing_disclosure_entries(
            &approach,
            confidence,
            Some(&baseline(100, 0.96)),
            &[],
            &execution(100, 5000, 120_000),
        );
        assert!(value(&full_size, DETECTABLE_RATE_KEY).is_none());
        assert!(value(&full_size, SAVED_FRACTION_KEY).is_none());

        let baseline_less = sizing_disclosure_entries(
            &approach,
            confidence,
            None,
            &[],
            &execution(100, 5000, 120_000),
        );
        assert!(value(&baseline_less, DETECTABLE_RATE_KEY).is_none());
    }

    #[test]
    fn detectable_rate_matches_the_sizing_statistics() {
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        let confidence = ConfidenceLevel::new(0.95);
        let entries = sizing_disclosure_entries(
            &approach,
            confidence,
            Some(&baseline(1000, 0.96)),
            &[],
            &execution(100, 5000, 0),
        );
        let disclosed: f64 = value(&entries, DETECTABLE_RATE_KEY)
            .unwrap()
            .parse()
            .unwrap();
        let expected =
            risk_driven_sizing::detectable_rate(100, 0.96, confidence, DEFAULT_TARGET_POWER);
        assert!((disclosed - expected).abs() < 1e-12);
        assert_eq!(value(&entries, DETECTABLE_POWER_KEY), Some("0.8"));
    }

    #[test]
    fn detectable_rate_runs_against_the_weakest_criterion_tally() {
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        let confidence = ConfidenceLevel::new(0.95);
        let tallies = vec![
            CriterionBaselineTally {
                criterion_name: "format valid".to_owned(),
                successes: 98,
                trials: 100,
            },
            CriterionBaselineTally {
                criterion_name: "content faithful".to_owned(),
                successes: 94,
                trials: 100,
            },
        ];
        let entries = sizing_disclosure_entries(
            &approach,
            confidence,
            Some(&baseline(1000, 0.96)),
            &tallies,
            &execution(100, 5000, 0),
        );
        let disclosed: f64 = value(&entries, DETECTABLE_RATE_KEY)
            .unwrap()
            .parse()
            .unwrap();
        let expected =
            risk_driven_sizing::detectable_rate(100, 0.94, confidence, DEFAULT_TARGET_POWER);
        assert!((disclosed - expected).abs() < 1e-12);
    }

    #[test]
    fn savings_derive_from_the_run_recorded_costs() {
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        // 100 samples over 5,000 ms and 120,000 tokens: 50 ms and 1,200
        // tokens per sample; 900 saved samples versus the baseline's 1,000.
        let entries = sizing_disclosure_entries(
            &approach,
            ConfidenceLevel::new(0.95),
            Some(&baseline(1000, 0.96)),
            &[],
            &execution(100, 5000, 120_000),
        );
        assert_eq!(value(&entries, SAVED_FRACTION_KEY), Some("0.9"));
        assert_eq!(value(&entries, TIME_SAVED_MS_KEY), Some("45000"));
        assert_eq!(value(&entries, TOKENS_SAVED_KEY), Some("1080000"));
    }

    #[test]
    fn token_half_degrades_away_when_no_tokens_are_recorded() {
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        let entries = sizing_disclosure_entries(
            &approach,
            ConfidenceLevel::new(0.95),
            Some(&baseline(1000, 0.96)),
            &[],
            &execution(100, 5000, 0),
        );
        assert!(value(&entries, TIME_SAVED_MS_KEY).is_some());
        assert!(value(&entries, TOKENS_SAVED_KEY).is_none());
    }

    #[test]
    fn a_perfect_baseline_rate_suppresses_the_downsizing_pair() {
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        let entries = sizing_disclosure_entries(
            &approach,
            ConfidenceLevel::new(0.95),
            Some(&baseline(1000, 1.0)),
            &[],
            &execution(100, 5000, 0),
        );
        assert_eq!(value(&entries, APPROACH_KEY), Some("sample-size-first"));
        assert!(value(&entries, DETECTABLE_RATE_KEY).is_none());
        assert!(value(&entries, SAVED_FRACTION_KEY).is_none());
    }
}
