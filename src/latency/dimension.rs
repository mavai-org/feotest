//! Latency dimension of a verdict record.
//!
//! Each resolved constraint is judged after the run on the latencies of the
//! samples that passed every functional criterion (Statistical Companion
//! §12.2.1), by the rule for its threshold source; the dimension's verdict
//! `V_latency` is the structural composite of every evaluation (§12.3.2).
//! The dimension's mode (§12.6) says whether `V_latency` binds the test: an
//! advisory dimension is judged exactly as an enforced one and reported, but
//! it never enters the test verdict.

use std::fmt;
use std::time::Duration;

use serde::{Serialize, Serializer};

use crate::latency::percentile::Percentile;
use crate::latency::resolver::{ConstraintSource, ResolvedLatencyConstraint, ThresholdProvenance};
use crate::model::types::optional_duration_as_millis;
use crate::statistics::decision::{Verdict, structural_composite};
use crate::statistics::latency::{
    self, ConstraintThreshold, LatencyCompliance, LatencyConstraint, LatencyJudgement,
    PrecedenceThreshold,
};
use crate::statistics::rules::{DecisionRule, EnforcementMode, TestIntent, alpha_from_confidence};

/// Per-evaluation status within the latency dimension: the outcome of the
/// constraint's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationStatus {
    /// The rule passed the constraint.
    Pass,
    /// The rule failed the constraint.
    Fail,
    /// Too few successful latencies to decide: no count can demonstrate an
    /// explicit requirement, or a baseline-derived percentile is degenerate
    /// under verification, or no successful latency arrived at all.
    Infeasible,
    /// A baseline-derived constraint for which no baseline rank achieves
    /// alpha at this test size: there is no threshold (§12.4.2).
    Saturated,
}

impl EvaluationStatus {
    /// The status's wire form.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Infeasible => "INFEASIBLE",
            Self::Saturated => "SATURATED",
        }
    }
}

impl Serialize for EvaluationStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// A single judged latency constraint.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyEvaluation {
    percentile: Percentile,
    #[serde(
        serialize_with = "optional_duration_as_millis",
        rename = "observedMs",
        skip_serializing_if = "Option::is_none"
    )]
    observed: Option<Duration>,
    #[serde(
        serialize_with = "optional_duration_as_millis",
        rename = "thresholdMs",
        skip_serializing_if = "Option::is_none"
    )]
    threshold: Option<Duration>,
    provenance: ThresholdProvenance,
    status: EvaluationStatus,
    confidence: f64,
    decision_rule: DecisionRule,
    verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    within_threshold: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_within: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    indicative: bool,
}

impl LatencyEvaluation {
    /// The percentile evaluated.
    #[must_use]
    pub const fn percentile(&self) -> Percentile {
        self.percentile
    }

    /// The observed nearest-rank percentile, if any latency arrived.
    #[must_use]
    pub const fn observed(&self) -> Option<Duration> {
        self.observed
    }

    /// The threshold judged against; `None` when saturated or underived.
    #[must_use]
    pub const fn threshold(&self) -> Option<Duration> {
        self.threshold
    }

    /// Where the threshold came from.
    #[must_use]
    pub const fn provenance(&self) -> ThresholdProvenance {
        self.provenance
    }

    /// The evaluation outcome.
    #[must_use]
    pub const fn status(&self) -> EvaluationStatus {
        self.status
    }

    /// The confidence level of the decision (`1 − alpha`).
    #[must_use]
    pub const fn confidence(&self) -> f64 {
        self.confidence
    }

    /// The rule that decided the evaluation, by its threshold source.
    #[must_use]
    pub const fn decision_rule(&self) -> DecisionRule {
        self.decision_rule
    }

    /// The evaluation's verdict under its rule.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// An explicit requirement's count of latencies at or below the
    /// threshold.
    #[must_use]
    pub const fn within_threshold(&self) -> Option<u32> {
        self.within_threshold
    }

    /// An explicit requirement's smallest count that demonstrates
    /// compliance; `None` when no count can.
    #[must_use]
    pub const fn required_within(&self) -> Option<u32> {
        self.required_within
    }

    /// Evaluated below the non-degeneracy minimum under smoke intent: a
    /// directional signal only.
    #[must_use]
    pub const fn indicative(&self) -> bool {
        self.indicative
    }

    /// The identifier a triggering list names this evaluation by.
    #[must_use]
    pub const fn constraint_id(&self) -> &'static str {
        self.percentile.label()
    }

    /// Creates a `LatencyEvaluation` from explicit parts, for testing
    /// renderers: the rule follows from the provenance and the verdict from
    /// the status.
    #[cfg(test)]
    #[must_use]
    pub const fn new(
        percentile: Percentile,
        observed: Option<Duration>,
        threshold: Option<Duration>,
        provenance: ThresholdProvenance,
        status: EvaluationStatus,
    ) -> Self {
        let decision_rule = match provenance {
            ThresholdProvenance::Explicit => DecisionRule::LatencyComplianceExactBinomial,
            ThresholdProvenance::BaselineDerived { .. } => DecisionRule::LatencyPrecedence,
        };
        let verdict = match status {
            EvaluationStatus::Pass => Verdict::Pass,
            EvaluationStatus::Fail => Verdict::Fail,
            EvaluationStatus::Infeasible | EvaluationStatus::Saturated => Verdict::Inconclusive,
        };
        Self {
            percentile,
            observed,
            threshold,
            provenance,
            status,
            confidence: 0.95,
            decision_rule,
            verdict,
            within_threshold: None,
            required_within: None,
            indicative: false,
        }
    }

    /// Judges one resolved constraint on the successful latencies.
    fn judge(
        constraint: &ResolvedLatencyConstraint,
        successful_latencies_ms: &[f64],
        intent: TestIntent,
    ) -> Self {
        let threshold = match constraint.source() {
            ConstraintSource::Explicit { threshold } => {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "millisecond thresholds fit in f64 mantissa"
                )]
                let ms = threshold.as_millis() as f64;
                ConstraintThreshold::Explicit(ms)
            }
            ConstraintSource::BaselineDerived {
                baseline_latencies_ms,
            } => ConstraintThreshold::BaselineDerived(baseline_latencies_ms),
        };
        let judgement = latency::judge_latency_constraint(
            successful_latencies_ms,
            &LatencyConstraint {
                percentile: constraint.percentile().as_fraction(),
                alpha: alpha_from_confidence(constraint.confidence()),
                threshold,
            },
            intent,
        );
        Self::from_judgement(constraint, &judgement)
    }

    /// Records a statistics-layer judgement as a verdict evaluation.
    fn from_judgement(
        constraint: &ResolvedLatencyConstraint,
        judgement: &LatencyJudgement,
    ) -> Self {
        let provenance = match constraint.source() {
            ConstraintSource::Explicit { .. } => ThresholdProvenance::Explicit,
            ConstraintSource::BaselineDerived {
                baseline_latencies_ms,
            } => ThresholdProvenance::BaselineDerived {
                confidence: constraint.confidence(),
                rank: judgement.precedence().and_then(PrecedenceThreshold::rank),
                n: u32::try_from(baseline_latencies_ms.len())
                    .expect("baseline latency count fits in u32"),
            },
        };
        Self {
            percentile: constraint.percentile(),
            observed: judgement.observed_ms().map(millis),
            threshold: judgement.threshold_ms().map(millis),
            provenance,
            status: status_of(judgement),
            confidence: constraint.confidence(),
            decision_rule: judgement.rule(),
            verdict: judgement.verdict(),
            within_threshold: judgement
                .compliance()
                .map(LatencyCompliance::within_threshold),
            required_within: judgement
                .compliance()
                .and_then(LatencyCompliance::minimum_within),
            indicative: judgement.indicative(),
        }
    }
}

/// A millisecond latency as a duration, rounded to the millisecond.
fn millis(ms: f64) -> Duration {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "latencies are non-negative milliseconds within u64"
    )]
    let whole = ms.round() as u64;
    Duration::from_millis(whole)
}

/// The evaluation status of a judgement: its rule's outcome, an
/// inconclusive one named by its cause.
fn status_of(judgement: &LatencyJudgement) -> EvaluationStatus {
    let saturated = judgement
        .precedence()
        .is_some_and(PrecedenceThreshold::saturated);
    match judgement.verdict() {
        Verdict::Pass => EvaluationStatus::Pass,
        Verdict::Fail => EvaluationStatus::Fail,
        Verdict::Inconclusive if saturated => EvaluationStatus::Saturated,
        Verdict::Inconclusive => EvaluationStatus::Infeasible,
    }
}

/// The latency dimension of a verdict record.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
// mavai-ref: JVI-ZCSHQ5K — do not remove (resolves in mavai-orchestrator)
pub struct LatencyDimension {
    mode: EnforcementMode,
    #[serde(serialize_with = "serialize_observed_percentiles")]
    observed_percentiles: Vec<(Percentile, Duration)>,
    evaluations: Vec<LatencyEvaluation>,
    successful_samples: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<Verdict>,
}

fn serialize_observed_percentiles<S: Serializer>(
    entries: &[(Percentile, Duration)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(entries.len()))?;
    for (p, d) in entries {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "observed latency in ms fits in u64"
        )]
        let ms = d.as_millis() as u64;
        map.serialize_entry(p.label(), &ms)?;
    }
    map.end()
}

impl LatencyDimension {
    /// Judges every resolved constraint on the successful latencies (those
    /// of the samples that passed every functional criterion) and composes
    /// their verdicts into `V_latency`. `mode` is the run's mode for the
    /// latency dimension; it changes nothing about the judgement.
    ///
    /// # Panics
    ///
    /// Panics if more than `u32::MAX` latencies are given.
    #[must_use]
    pub fn build(
        successful_latencies_ms: &[f64],
        constraints: &[ResolvedLatencyConstraint],
        intent: TestIntent,
        mode: EnforcementMode,
    ) -> Self {
        let successful_samples =
            u32::try_from(successful_latencies_ms.len()).expect("sample count fits in u32");
        let evaluations: Vec<LatencyEvaluation> = constraints
            .iter()
            .map(|c| LatencyEvaluation::judge(c, successful_latencies_ms, intent))
            .collect();
        Self::from_evaluations(evaluations, successful_samples, mode)
    }

    /// Assembles the dimension from its evaluations.
    fn from_evaluations(
        evaluations: Vec<LatencyEvaluation>,
        successful_samples: u32,
        mode: EnforcementMode,
    ) -> Self {
        let mut observed_percentiles: Vec<(Percentile, Duration)> = Vec::new();
        for evaluation in &evaluations {
            if let Some(observed) = evaluation.observed
                && !observed_percentiles
                    .iter()
                    .any(|(p, _)| *p == evaluation.percentile)
            {
                observed_percentiles.push((evaluation.percentile, observed));
            }
        }
        let verdict = structural_composite(evaluations.iter().map(|e| e.verdict));
        Self {
            mode,
            observed_percentiles,
            evaluations,
            successful_samples,
            verdict,
        }
    }

    /// Whether the dimension binds the test (enforced) or is reported only
    /// (advisory).
    #[must_use]
    pub const fn mode(&self) -> EnforcementMode {
        self.mode
    }

    /// `V_latency`: the structural composite of every evaluation, whatever
    /// the mode; `None` when the test carries no latency constraint.
    #[must_use]
    pub const fn verdict(&self) -> Option<Verdict> {
        self.verdict
    }

    /// Whether `V_latency` is PASS (vacuously, with no constraint). Says
    /// nothing about whether the dimension binds: see [`mode`](Self::mode).
    #[must_use]
    pub fn passed(&self) -> bool {
        self.verdict.is_none_or(|v| v == Verdict::Pass)
    }

    /// Per-percentile observed values, one entry per percentile evaluated.
    #[must_use]
    pub fn observed_percentiles(&self) -> &[(Percentile, Duration)] {
        &self.observed_percentiles
    }

    /// The evaluations performed, in percentile order.
    #[must_use]
    pub fn evaluations(&self) -> &[LatencyEvaluation] {
        &self.evaluations
    }

    /// Number of successful latencies judged.
    #[must_use]
    pub const fn successful_samples(&self) -> u32 {
        self.successful_samples
    }

    /// The `(constraint id, verdict)` pair of every evaluation, for the test
    /// verdict's composition.
    #[must_use]
    pub fn constraint_verdicts(&self) -> Vec<(String, Verdict)> {
        self.evaluations
            .iter()
            .map(|e| (e.constraint_id().to_owned(), e.verdict))
            .collect()
    }

    /// The `(rule, alpha)` of every evaluation, for the Type-I envelopes of
    /// an enforced dimension.
    pub(crate) fn decisions(&self) -> impl Iterator<Item = (DecisionRule, f64)> + '_ {
        self.evaluations
            .iter()
            .map(|e| (e.decision_rule, alpha_from_confidence(e.confidence)))
    }

    /// Creates an enforced `LatencyDimension` from evaluations, for testing
    /// renderers.
    #[cfg(test)]
    #[must_use]
    pub fn from_parts(evaluations: Vec<LatencyEvaluation>, successful_samples: u32) -> Self {
        Self::from_evaluations(evaluations, successful_samples, EnforcementMode::Enforced)
    }

    /// The same dimension under another mode, for testing renderers.
    #[cfg(test)]
    #[must_use]
    pub const fn with_mode(mut self, mode: EnforcementMode) -> Self {
        self.mode = mode;
        self
    }
}

impl fmt::Display for LatencyDimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Latency dimension ({}, {} successful samples):",
            self.mode, self.successful_samples
        )?;
        for ev in &self.evaluations {
            let obs = ev
                .observed
                .map_or_else(|| "—".to_string(), |d| format!("{} ms", d.as_millis()));
            let thr = ev
                .threshold
                .map_or_else(|| "none".to_string(), |d| format!("{} ms", d.as_millis()));
            let provenance = match ev.provenance {
                ThresholdProvenance::Explicit => "explicit".to_string(),
                ThresholdProvenance::BaselineDerived {
                    confidence,
                    rank,
                    n,
                } => rank.map_or_else(
                    || format!("baseline saturated n={n} c={confidence:.2}"),
                    |rank| format!("baseline rank={rank}/{n} c={confidence:.2}"),
                ),
            };
            writeln!(
                f,
                "  {}: observed={obs}, threshold={thr} [{provenance}; {}] -> {}",
                ev.percentile,
                ev.decision_rule,
                ev.status.name()
            )?;
        }
        if self.mode == EnforcementMode::Advisory {
            writeln!(f, "  advisory: reported, does not fail the test")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn explicit(percentile: Percentile, ms: u64) -> ResolvedLatencyConstraint {
        crate::latency::resolver::resolve(
            &crate::latency::LatencyCriterion::meeting()
                .at_most(percentile, Duration::from_millis(ms)),
            None,
            crate::latency::resolver::ConstraintConfidence {
                explicit: 0.95,
                baseline: 0.95,
            },
        )
        .remove(0)
    }

    #[test]
    fn an_explicit_requirement_is_decided_by_the_exact_binomial_rule() {
        let latencies: Vec<f64> = (1..=100).map(f64::from).collect();
        let dimension = LatencyDimension::build(
            &latencies,
            &[explicit(Percentile::P95, 99)],
            TestIntent::Verification,
            EnforcementMode::Enforced,
        );
        let evaluation = &dimension.evaluations()[0];
        assert_eq!(
            evaluation.decision_rule(),
            DecisionRule::LatencyComplianceExactBinomial
        );
        assert_eq!(evaluation.within_threshold(), Some(99));
        assert_eq!(evaluation.status(), EvaluationStatus::Pass);
        assert_eq!(dimension.verdict(), Some(Verdict::Pass));
        assert!(dimension.passed());
    }

    #[test]
    fn too_few_latencies_for_an_explicit_requirement_are_infeasible() {
        let latencies: Vec<f64> = (1..=10).map(f64::from).collect();
        let dimension = LatencyDimension::build(
            &latencies,
            &[explicit(Percentile::P95, 1000)],
            TestIntent::Verification,
            EnforcementMode::Enforced,
        );
        assert_eq!(
            dimension.evaluations()[0].status(),
            EvaluationStatus::Infeasible
        );
        assert_eq!(dimension.verdict(), Some(Verdict::Inconclusive));
        assert!(!dimension.passed());
    }

    #[test]
    fn a_saturated_baseline_constraint_has_no_threshold() {
        let baseline: Vec<u64> = (1..=100).collect();
        let constraints = crate::latency::resolver::resolve(
            &crate::latency::LatencyCriterion::empirical().against_baseline(Percentile::P95),
            Some(&crate::spec::baseline::LatencyBlock {
                latencies_ms: baseline,
                mean_ms: 50,
                max_ms: 100,
            }),
            crate::latency::resolver::ConstraintConfidence {
                explicit: 0.95,
                baseline: 0.95,
            },
        );
        let latencies: Vec<f64> = (1..=15).map(f64::from).collect();
        let dimension = LatencyDimension::build(
            &latencies,
            &constraints,
            TestIntent::Verification,
            EnforcementMode::Enforced,
        );
        let p95 = dimension
            .evaluations()
            .iter()
            .find(|e| e.percentile() == Percentile::P95)
            .unwrap();
        assert_eq!(p95.status(), EvaluationStatus::Saturated);
        assert_eq!(p95.threshold(), None);
        assert_eq!(dimension.verdict(), Some(Verdict::Inconclusive));
    }

    #[test]
    fn an_advisory_dimension_is_judged_exactly_as_an_enforced_one() {
        let latencies: Vec<f64> = (1..=100).map(f64::from).collect();
        let constraints = [explicit(Percentile::P95, 50)];
        let enforced = LatencyDimension::build(
            &latencies,
            &constraints,
            TestIntent::Verification,
            EnforcementMode::Enforced,
        );
        let advisory = LatencyDimension::build(
            &latencies,
            &constraints,
            TestIntent::Verification,
            EnforcementMode::Advisory,
        );
        assert_eq!(advisory.mode(), EnforcementMode::Advisory);
        assert_eq!(advisory.evaluations(), enforced.evaluations());
        assert_eq!(advisory.verdict(), Some(Verdict::Fail));
        assert_eq!(advisory.evaluations()[0].status(), EvaluationStatus::Fail);
        assert_eq!(
            advisory.constraint_verdicts(),
            [("p95".to_owned(), Verdict::Fail)]
        );
    }

    #[test]
    fn the_display_names_the_mode_and_each_rule() {
        let evaluation = LatencyEvaluation::new(
            Percentile::P99,
            Some(Duration::from_millis(520)),
            Some(Duration::from_millis(500)),
            ThresholdProvenance::BaselineDerived {
                confidence: 0.95,
                rank: Some(99),
                n: 100,
            },
            EvaluationStatus::Fail,
        );
        let advisory = LatencyDimension::from_parts(vec![evaluation], 100)
            .with_mode(EnforcementMode::Advisory);
        let shown = advisory.to_string();
        assert!(shown.starts_with("Latency dimension (advisory, 100 successful samples):"));
        assert!(shown.contains("[baseline rank=99/100 c=0.95; latency/precedence] -> FAIL"));
        assert!(shown.contains("advisory: reported, does not fail the test"));
        let enforced = LatencyDimension::from_parts(vec![evaluation], 100).to_string();
        assert!(!enforced.contains("advisory"));
    }

    #[test]
    fn latency_dimension_serialises_with_ms_durations() {
        let evaluation_pass = LatencyEvaluation::new(
            Percentile::P95,
            Some(Duration::from_millis(180)),
            Some(Duration::from_millis(200)),
            ThresholdProvenance::Explicit,
            EvaluationStatus::Pass,
        );
        let evaluation_fail = LatencyEvaluation::new(
            Percentile::P99,
            Some(Duration::from_millis(520)),
            Some(Duration::from_millis(500)),
            ThresholdProvenance::BaselineDerived {
                confidence: 0.95,
                rank: Some(99),
                n: 100,
            },
            EvaluationStatus::Fail,
        );
        let dimension = LatencyDimension::from_parts(vec![evaluation_pass, evaluation_fail], 100)
            .with_mode(EnforcementMode::Advisory);

        insta::assert_json_snapshot!(
            "latency_dimension",
            serde_json::to_value(&dimension).unwrap()
        );
    }
}
