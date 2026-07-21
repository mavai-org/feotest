//! Normative judgement at experiment time.
//!
//! A measure experiment over a contract that declares normative criteria
//! (`Criterion::meeting().pass_rate(..)`) already holds everything needed to
//! judge the run against each stipulated threshold: the per-criterion
//! tallies, the run's own sample count, and a threshold whose validity does
//! not depend on any baseline. Each normative criterion is judged by
//! comparing the one-sided Wilson lower bound of its observed pass rate — at
//! the run's sample count and the framework's default confidence — against
//! the stipulated threshold. Empirical criteria are never judged at
//! experiment time: their bar does not exist until a baseline supplies it.
//!
//! A judgement states one fact — the relation of this run's evidence to a
//! stipulation in force at measure time — and implies nothing further about
//! the service under test. A failed judgement can be entirely expected: an
//! aspirational bar measured mid-development, a fresh configuration
//! characterised before tuning, a service measured precisely because it is
//! suspected to sit below its bar.

use std::fmt;

use crate::criteria::{CriteriaCounts, CriterionTarget};
use crate::spec::baseline::{NormativeJudgementBlock, NormativeJudgementState};
use crate::statistics::types::ConfidenceLevel;
use crate::statistics::{defaults, feasibility, proportion};

/// The three-valued state of one normative judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgementState {
    /// The run's evidence clears the stipulated threshold.
    Met,
    /// The run's evidence does not clear the stipulated threshold.
    Failed,
    /// The run's sample count cannot support the stipulated threshold at the
    /// judgement confidence, even with a perfect observation.
    Unsupportable {
        /// The smallest sample count at which a perfect observation would
        /// clear the stipulated threshold.
        feasible_minimum_samples: u32,
    },
}

/// The judgement of one normative criterion against its stipulated threshold,
/// rendered from a measure run's own samples.
#[derive(Debug, Clone, PartialEq)]
pub struct NormativeJudgement {
    criterion: String,
    state: JudgementState,
    stipulated_threshold: f64,
    confidence: f64,
    samples: u32,
    observed_rate: Option<f64>,
    lower_bound: Option<f64>,
}

impl NormativeJudgement {
    /// The name of the judged criterion.
    #[must_use]
    pub fn criterion(&self) -> &str {
        &self.criterion
    }

    /// The judgement state.
    #[must_use]
    pub const fn state(&self) -> JudgementState {
        self.state
    }

    /// The stipulated threshold the run was judged against.
    #[must_use]
    pub const fn stipulated_threshold(&self) -> f64 {
        self.stipulated_threshold
    }

    /// The confidence level of the judgement.
    #[must_use]
    pub const fn confidence(&self) -> f64 {
        self.confidence
    }

    /// The number of samples the judgement drew on.
    #[must_use]
    pub const fn samples(&self) -> u32 {
        self.samples
    }

    /// The criterion's observed pass rate, or `None` when no samples were
    /// recorded for it.
    #[must_use]
    pub const fn observed_rate(&self) -> Option<f64> {
        self.observed_rate
    }

    /// The one-sided Wilson lower bound of the observed rate at the run's
    /// sample count, or `None` when no samples were recorded.
    #[must_use]
    pub const fn lower_bound(&self) -> Option<f64> {
        self.lower_bound
    }

    /// Converts the judgement into its baseline-spec marker.
    #[must_use]
    pub(crate) const fn to_spec_block(&self) -> NormativeJudgementBlock {
        let (state, feasible_minimum_samples) = match self.state {
            JudgementState::Met => (NormativeJudgementState::Met, None),
            JudgementState::Failed => (NormativeJudgementState::Failed, None),
            JudgementState::Unsupportable {
                feasible_minimum_samples,
            } => (
                NormativeJudgementState::Unsupportable,
                Some(feasible_minimum_samples),
            ),
        };
        NormativeJudgementBlock {
            state,
            stipulated_threshold: self.stipulated_threshold,
            confidence: self.confidence,
            feasible_minimum_samples,
        }
    }
}

impl fmt::Display for NormativeJudgement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let confidence_pct = self.confidence * 100.0;
        match self.state {
            JudgementState::Met => write!(
                f,
                "normative judgement: MET — the {confidence_pct:.0}%-confident lower bound \
                 {lb:.4} clears the stipulated threshold {threshold}",
                lb = self.lower_bound.unwrap_or(0.0),
                threshold = self.stipulated_threshold,
            ),
            JudgementState::Failed => write!(
                f,
                "NORMATIVE JUDGEMENT: FAILED — the {confidence_pct:.0}%-confident lower bound \
                 {lb:.4} does not clear the stipulated threshold {threshold}",
                lb = self.lower_bound.unwrap_or(0.0),
                threshold = self.stipulated_threshold,
            ),
            JudgementState::Unsupportable {
                feasible_minimum_samples,
            } => write!(
                f,
                "NORMATIVE JUDGEMENT: UNSUPPORTABLE at this sample size — {samples} samples \
                 cannot support the stipulated threshold {threshold} at {confidence_pct:.0}% \
                 confidence (feasible minimum: {feasible_minimum_samples} samples)",
                samples = self.samples,
                threshold = self.stipulated_threshold,
            ),
        }
    }
}

/// Judges every normative criterion among `targets` against its stipulated
/// threshold, using the run's own per-criterion tallies. Criteria with
/// empirical or zero-failures targets yield no judgement — by definition,
/// their bar does not exist at experiment time.
// mavai-ref: JVI-305FCX1 — do not remove (resolves in mavai-orchestrator)
pub fn judge_normative_criteria(
    targets: &[(&str, &CriterionTarget)],
    counts: &CriteriaCounts,
) -> Vec<NormativeJudgement> {
    targets
        .iter()
        .filter_map(|(name, target)| match target {
            CriterionTarget::NormativeRate(rate) => Some(judge_criterion(name, *rate, counts)),
            CriterionTarget::EmpiricalRate | CriterionTarget::ZeroFailures => None,
        })
        .collect()
}

/// Judges one normative criterion: unsupportable when the tally cannot carry
/// the stipulated threshold at the judgement confidence, otherwise met or
/// failed by whether the Wilson lower bound clears the stipulation.
fn judge_criterion(name: &str, stipulated: f64, counts: &CriteriaCounts) -> NormativeJudgement {
    let confidence = ConfidenceLevel::new(defaults::DEFAULT_CONFIDENCE);
    let (pass, total) = counts
        .get(name)
        .map_or((0, 0), |tally| (tally.pass(), tally.total()));

    let observed_rate = (total > 0).then(|| f64::from(pass) / f64::from(total));
    let lower_bound = (total > 0).then(|| proportion::lower_bound(pass, total, confidence));

    let feasibility = feasibility::feasibility_check(total, stipulated, confidence);
    let state = if feasibility.feasible() {
        // Feasibility implies total > 0, so the lower bound exists.
        if lower_bound.expect("a feasible tally has samples") >= stipulated {
            JudgementState::Met
        } else {
            JudgementState::Failed
        }
    } else {
        JudgementState::Unsupportable {
            feasible_minimum_samples: feasibility.minimum_samples(),
        }
    };

    NormativeJudgement {
        criterion: name.to_owned(),
        state,
        stipulated_threshold: stipulated,
        confidence: confidence.value(),
        samples: total,
        observed_rate,
        lower_bound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::criteria::CriteriaCounts;
    use crate::criteria::CriterionSampleResult;
    use crate::model::ContractViolation;

    fn counts_of(criterion: &str, passes: u32, fails: u32) -> CriteriaCounts {
        let mut counts = CriteriaCounts::new();
        for _ in 0..passes {
            counts.record_sample(&[CriterionSampleResult::pass(criterion)]);
        }
        for _ in 0..fails {
            counts.record_sample(&[CriterionSampleResult::fail(
                criterion,
                ContractViolation::new("check", "reason"),
            )]);
        }
        counts
    }

    #[test]
    fn clearing_the_stipulation_is_met() {
        let counts = counts_of("c", 100, 0);
        let target = CriterionTarget::NormativeRate(0.5);
        let judgements = judge_normative_criteria(&[("c", &target)], &counts);
        assert_eq!(judgements.len(), 1);
        assert_eq!(judgements[0].state(), JudgementState::Met);
        assert!(judgements[0].lower_bound().unwrap() >= 0.5);
    }

    #[test]
    fn not_clearing_the_stipulation_is_failed() {
        let counts = counts_of("c", 60, 40);
        let target = CriterionTarget::NormativeRate(0.9);
        let judgements = judge_normative_criteria(&[("c", &target)], &counts);
        assert_eq!(judgements[0].state(), JudgementState::Failed);
        assert!((judgements[0].observed_rate().unwrap() - 0.6).abs() < 1e-12);
        assert!(judgements[0].lower_bound().unwrap() < 0.9);
    }

    #[test]
    fn undersized_run_is_unsupportable_with_feasible_minimum() {
        let counts = counts_of("c", 10, 0);
        let target = CriterionTarget::NormativeRate(0.99);
        let judgements = judge_normative_criteria(&[("c", &target)], &counts);
        let JudgementState::Unsupportable {
            feasible_minimum_samples,
        } = judgements[0].state()
        else {
            panic!("expected unsupportable, got {:?}", judgements[0].state());
        };
        assert!(feasible_minimum_samples > 10);
    }

    #[test]
    fn empirical_and_zero_failures_criteria_are_not_judged() {
        let counts = counts_of("e", 10, 0);
        let empirical = CriterionTarget::EmpiricalRate;
        let observational = CriterionTarget::ZeroFailures;
        let judgements =
            judge_normative_criteria(&[("e", &empirical), ("z", &observational)], &counts);
        assert!(judgements.is_empty());
    }

    #[test]
    fn judgement_confidence_is_the_framework_default() {
        let counts = counts_of("c", 100, 0);
        let target = CriterionTarget::NormativeRate(0.5);
        let judgements = judge_normative_criteria(&[("c", &target)], &counts);
        assert!((judgements[0].confidence() - defaults::DEFAULT_CONFIDENCE).abs() < 1e-12);
    }

    #[test]
    fn unseen_criterion_is_unsupportable() {
        let counts = CriteriaCounts::new();
        let target = CriterionTarget::NormativeRate(0.9);
        let judgements = judge_normative_criteria(&[("missing", &target)], &counts);
        assert!(matches!(
            judgements[0].state(),
            JudgementState::Unsupportable { .. }
        ));
        assert!(judgements[0].observed_rate().is_none());
        assert!(judgements[0].lower_bound().is_none());
    }

    #[test]
    fn spec_block_carries_state_threshold_and_confidence() {
        let counts = counts_of("c", 60, 40);
        let target = CriterionTarget::NormativeRate(0.9);
        let judgements = judge_normative_criteria(&[("c", &target)], &counts);
        let block = judgements[0].to_spec_block();
        assert_eq!(block.state, NormativeJudgementState::Failed);
        assert!((block.stipulated_threshold - 0.9).abs() < 1e-12);
        assert!((block.confidence - 0.95).abs() < 1e-12);
        assert!(block.feasible_minimum_samples.is_none());
    }

    #[test]
    fn unsupportable_spec_block_carries_feasible_minimum() {
        let counts = counts_of("c", 5, 0);
        let target = CriterionTarget::NormativeRate(0.99);
        let judgements = judge_normative_criteria(&[("c", &target)], &counts);
        let block = judgements[0].to_spec_block();
        assert_eq!(block.state, NormativeJudgementState::Unsupportable);
        assert!(block.feasible_minimum_samples.unwrap() > 5);
    }

    #[test]
    fn display_wording_relates_to_the_stipulation() {
        let counts = counts_of("c", 60, 40);
        let target = CriterionTarget::NormativeRate(0.9);
        let judgements = judge_normative_criteria(&[("c", &target)], &counts);
        let rendered = judgements[0].to_string();
        assert!(rendered.contains("FAILED"));
        assert!(rendered.contains("does not clear the stipulated threshold 0.9"));
        // The judgement is a relation to the stipulation, never a claim
        // about the service's validity.
        assert!(!rendered.to_lowercase().contains("invalid"));
    }

    #[test]
    fn display_states_the_feasible_minimum_when_unsupportable() {
        let counts = counts_of("c", 10, 0);
        let target = CriterionTarget::NormativeRate(0.99);
        let judgements = judge_normative_criteria(&[("c", &target)], &counts);
        let rendered = judgements[0].to_string();
        assert!(rendered.contains("UNSUPPORTABLE"));
        assert!(rendered.contains("feasible minimum"));
    }
}
