//! Decisions: one criterion under its rule, and their composition
//! (Statistical Companion §1.4.6, §12.3.2).
//!
//! A criterion is decided by the rule its threshold's origin selects:
//!
//! - a **given** (normative) requirement by `compliance/exact-binomial` —
//!   [`evaluate_compliance`];
//! - a **baseline-derived** (empirical) bar by `regression/fisher` —
//!   [`evaluate_regression`].
//!
//! Criteria compose by one structural rule — PASS if every verdict passes,
//! FAIL if any fails, INCONCLUSIVE otherwise — and the same rule composes
//! the functional dimension `V_rate` with the latency dimension `V_latency`
//! into the test's verdict `V_test`. A FAIL or an INCONCLUSIVE names what
//! decided it. The Type-I envelopes are disclosed by procedure direction:
//! the sum of alpha over the compliance decisions (false compliance) and
//! over the regression decisions (false degradation signal).

use std::fmt;

use serde::{Serialize, Serializer};

use crate::statistics::compliance::{clopper_pearson_lower, minimum_passing_count};
use crate::statistics::distributions::binomial_upper_tail;
use crate::statistics::regression::{RegressionDerivation, derive_regression_cutoff};
use crate::statistics::rules::{DecisionRule, Direction};

/// The outcome of a criterion, a dimension, or a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The evidence supports the claim at the configured level.
    Pass,

    /// The claim is not supported: a degradation was signalled, or
    /// compliance was not demonstrated.
    Fail,

    /// The data cannot decide the question — too few samples to judge, or
    /// no threshold exists at this size.
    Inconclusive,
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(f, "PASS"),
            Self::Fail => write!(f, "FAIL"),
            Self::Inconclusive => write!(f, "INCONCLUSIVE"),
        }
    }
}

impl Serialize for Verdict {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// PASS if every verdict passes, FAIL if any fails, INCONCLUSIVE otherwise;
/// `None` when there is nothing to compose.
#[must_use]
pub fn structural_composite(verdicts: impl IntoIterator<Item = Verdict>) -> Option<Verdict> {
    let mut composite: Option<Verdict> = None;
    for verdict in verdicts {
        composite = Some(match (composite, verdict) {
            (_, Verdict::Fail) | (Some(Verdict::Fail), _) => Verdict::Fail,
            (_, Verdict::Inconclusive) | (Some(Verdict::Inconclusive), _) => Verdict::Inconclusive,
            (None | Some(Verdict::Pass), Verdict::Pass) => Verdict::Pass,
        });
    }
    composite
}

/// A criterion decided by `compliance/exact-binomial`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComplianceDecision {
    verdict: Verdict,
    successes: u32,
    trials: u32,
    requirement: f64,
    alpha: f64,
    minimum_passing: Option<u32>,
    false_compliance: f64,
    clopper_pearson_lower: f64,
}

impl ComplianceDecision {
    /// The rule that decided it.
    pub const RULE: DecisionRule = DecisionRule::ComplianceExactBinomial;

    /// PASS iff `K ≥ k_min`. FAIL means compliance was not demonstrated —
    /// including when no count can pass at this size, where the FAIL was
    /// certain before the run and carries no evidence about the service.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// `K`.
    #[must_use]
    pub const fn successes(&self) -> u32 {
        self.successes
    }

    /// `n`.
    #[must_use]
    pub const fn trials(&self) -> u32 {
        self.trials
    }

    /// `p_req`.
    #[must_use]
    pub const fn requirement(&self) -> f64 {
        self.requirement
    }

    /// The one-sided level.
    #[must_use]
    pub const fn alpha(&self) -> f64 {
        self.alpha
    }

    /// `k_min`; `None` when no count can pass.
    #[must_use]
    pub const fn minimum_passing(&self) -> Option<u32> {
        self.minimum_passing
    }

    /// Whether any outcome of this size could have passed.
    #[must_use]
    pub const fn pass_possible(&self) -> bool {
        self.minimum_passing.is_some()
    }

    /// `P_{p_req}(K ≥ k_min)`, 0 when no count can pass.
    #[must_use]
    pub const fn false_compliance(&self) -> f64 {
        self.false_compliance
    }

    /// The one-sided Clopper–Pearson lower bound at `1 − alpha`, reported
    /// beside the verdict; it decides nothing.
    #[must_use]
    pub const fn clopper_pearson_lower(&self) -> f64 {
        self.clopper_pearson_lower
    }
}

/// Decides a criterion against a given requirement.
///
/// # Panics
///
/// Panics if `trials` is zero, `successes` exceeds it, or the requirement or
/// level is outside `(0, 1)`.
#[must_use]
pub fn evaluate_compliance(
    successes: u32,
    trials: u32,
    requirement: f64,
    alpha: f64,
) -> ComplianceDecision {
    assert!(successes <= trials, "successes must not exceed trials");
    let minimum_passing = minimum_passing_count(requirement, trials, alpha);
    let passed = minimum_passing.is_some_and(|k_min| successes >= k_min);
    ComplianceDecision {
        verdict: if passed { Verdict::Pass } else { Verdict::Fail },
        successes,
        trials,
        requirement,
        alpha,
        minimum_passing,
        false_compliance: minimum_passing.map_or(0.0, |k_min| {
            binomial_upper_tail(i64::from(k_min), trials, requirement)
        }),
        clopper_pearson_lower: clopper_pearson_lower(successes, trials, alpha),
    }
}

/// A criterion decided by `regression/fisher`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegressionDecision {
    verdict: Verdict,
    successes: u32,
    trials: u32,
    baseline_successes: u32,
    baseline_trials: u32,
    derivation: RegressionDerivation,
}

impl RegressionDecision {
    /// The rule that decided it.
    pub const RULE: DecisionRule = DecisionRule::RegressionFisher;

    /// PASS iff `K_t ≥ cutoff`.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// `K_t`.
    #[must_use]
    pub const fn successes(&self) -> u32 {
        self.successes
    }

    /// `n_t`.
    #[must_use]
    pub const fn trials(&self) -> u32 {
        self.trials
    }

    /// `K_b`.
    #[must_use]
    pub const fn baseline_successes(&self) -> u32 {
        self.baseline_successes
    }

    /// `n_b`.
    #[must_use]
    pub const fn baseline_trials(&self) -> u32 {
        self.baseline_trials
    }

    /// The cutoff and what the report discloses about it.
    #[must_use]
    pub const fn derivation(&self) -> &RegressionDerivation {
        &self.derivation
    }

    /// The binding decision artefact.
    #[must_use]
    pub const fn cutoff(&self) -> u32 {
        self.derivation.cutoff()
    }

    /// The one-sided level.
    #[must_use]
    pub const fn alpha(&self) -> f64 {
        self.derivation.alpha()
    }
}

/// Decides a criterion against its baseline.
///
/// The design rule that a test may not exceed its baseline is judged before
/// the run ([`check_test_size`](crate::statistics::rules::check_test_size)),
/// not here.
///
/// # Panics
///
/// Panics if `successes` exceeds `trials`, or on the inputs
/// [`fisher_cutoff`](crate::statistics::regression::fisher_cutoff) rejects.
#[must_use]
pub fn evaluate_regression(
    successes: u32,
    trials: u32,
    baseline_successes: u32,
    baseline_trials: u32,
    alpha: f64,
) -> RegressionDecision {
    assert!(successes <= trials, "successes must not exceed trials");
    let derivation = derive_regression_cutoff(baseline_successes, baseline_trials, trials, alpha);
    RegressionDecision {
        verdict: if successes >= derivation.cutoff() {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        successes,
        trials,
        baseline_successes,
        baseline_trials,
        derivation,
    }
}

/// The union-bound Type-I envelopes, split by direction; each is `None`
/// when the test makes no decision of that direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Envelopes {
    false_compliance: Option<f64>,
    false_degradation_signal: Option<f64>,
}

impl Envelopes {
    /// The sum of alpha over the compliance-direction decisions.
    #[must_use]
    pub const fn false_compliance(&self) -> Option<f64> {
        self.false_compliance
    }

    /// The sum of alpha over the regression-direction decisions.
    #[must_use]
    pub const fn false_degradation_signal(&self) -> Option<f64> {
        self.false_degradation_signal
    }
}

/// Sums each direction's alphas over the `(rule, alpha)` decisions made.
#[must_use]
pub fn type_one_envelopes(decisions: impl IntoIterator<Item = (DecisionRule, f64)>) -> Envelopes {
    let mut envelopes = Envelopes {
        false_compliance: None,
        false_degradation_signal: None,
    };
    for (rule, alpha) in decisions {
        let slot = match rule.direction() {
            Direction::Compliance => &mut envelopes.false_compliance,
            Direction::Regression => &mut envelopes.false_degradation_signal,
        };
        *slot = Some(slot.unwrap_or(0.0) + alpha);
    }
    envelopes
}

/// What decided a test's FAIL or INCONCLUSIVE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    /// A functional criterion.
    Criterion,
    /// An enforced latency constraint.
    Latency,
}

impl TriggerKind {
    /// The kind's name, as reports state it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Criterion => "criterion",
            Self::Latency => "latency",
        }
    }
}

/// One functional criterion or enforced latency constraint that decided the
/// test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Trigger {
    #[serde(serialize_with = "serialize_trigger_kind")]
    kind: TriggerKind,
    id: String,
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's serialize_with hands the field by reference"
)]
fn serialize_trigger_kind<S: Serializer>(
    kind: &TriggerKind,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(kind.name())
}

impl Trigger {
    /// Whether a criterion or a latency constraint decided.
    #[must_use]
    pub const fn kind(&self) -> TriggerKind {
        self.kind
    }

    /// The criterion's or the constraint's identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// The test's verdict `V_test` and the two dimensions it composes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverallVerdict {
    rate_verdict: Option<Verdict>,
    latency_verdict: Option<Verdict>,
    verdict: Verdict,
    triggering: Vec<Trigger>,
}

impl OverallVerdict {
    /// `V_rate`, the composite of the functional criteria; `None` for a test
    /// with none.
    #[must_use]
    pub const fn rate_verdict(&self) -> Option<Verdict> {
        self.rate_verdict
    }

    /// `V_latency`, the composite of the enforced latency constraints;
    /// `None` for a test that enforces none.
    #[must_use]
    pub const fn latency_verdict(&self) -> Option<Verdict> {
        self.latency_verdict
    }

    /// `V_test`, the composite of the dimensions present.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// For a FAIL or an INCONCLUSIVE, the criteria and enforced constraints
    /// whose verdict is the test's, criteria first.
    #[must_use]
    pub fn triggering(&self) -> &[Trigger] {
        &self.triggering
    }
}

/// Composes `V_test` from `(id, verdict)` pairs of the functional criteria
/// and of the enforced latency constraints (advisory ones never enter).
///
/// # Panics
///
/// Panics when there is neither a criterion nor an enforced latency
/// constraint to compose.
#[must_use]
pub fn compose_overall_verdict(
    criteria: &[(String, Verdict)],
    latency: &[(String, Verdict)],
) -> OverallVerdict {
    let rate_verdict = structural_composite(criteria.iter().map(|(_, v)| *v));
    let latency_verdict = structural_composite(latency.iter().map(|(_, v)| *v));
    let verdict = structural_composite(rate_verdict.into_iter().chain(latency_verdict))
        .expect("a test verdict needs a criterion or an enforced latency constraint");
    let triggering = if verdict == Verdict::Pass {
        Vec::new()
    } else {
        let named = |kind: TriggerKind, pairs: &[(String, Verdict)]| -> Vec<Trigger> {
            pairs
                .iter()
                .filter(|(_, v)| *v == verdict)
                .map(|(id, _)| Trigger {
                    kind,
                    id: id.clone(),
                })
                .collect()
        };
        let mut triggers = named(TriggerKind::Criterion, criteria);
        triggers.extend(named(TriggerKind::Latency, latency));
        triggers
    };
    OverallVerdict {
        rate_verdict,
        latency_verdict,
        verdict,
        triggering,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn pairs(entries: &[(&str, Verdict)]) -> Vec<(String, Verdict)> {
        entries
            .iter()
            .map(|(id, v)| ((*id).to_owned(), *v))
            .collect()
    }

    #[test]
    fn verdict_display() {
        assert_eq!(Verdict::Pass.to_string(), "PASS");
        assert_eq!(Verdict::Fail.to_string(), "FAIL");
        assert_eq!(Verdict::Inconclusive.to_string(), "INCONCLUSIVE");
    }

    #[test]
    fn composite_fails_on_any_fail_and_is_otherwise_inconclusive() {
        use Verdict::{Fail, Inconclusive, Pass};
        assert_eq!(structural_composite([Pass, Pass]), Some(Pass));
        assert_eq!(
            structural_composite([Pass, Inconclusive]),
            Some(Inconclusive)
        );
        assert_eq!(structural_composite([Inconclusive, Fail]), Some(Fail));
        assert_eq!(structural_composite([Fail, Inconclusive]), Some(Fail));
        assert_eq!(structural_composite([]), None);
    }

    #[test]
    fn compliance_demonstrated_at_k_min() {
        let decision = evaluate_compliance(49, 50, 0.90, 0.05);
        assert_eq!(decision.verdict(), Verdict::Pass);
        let short = evaluate_compliance(48, 50, 0.90, 0.05);
        assert_eq!(short.verdict(), Verdict::Fail);
        assert_eq!(decision.minimum_passing(), Some(49));
    }

    #[test]
    fn an_infeasible_compliance_design_cannot_pass() {
        let decision = evaluate_compliance(50, 50, 0.95, 0.05);
        assert_eq!(decision.verdict(), Verdict::Fail);
        assert!(!decision.pass_possible());
        assert_relative_eq!(decision.false_compliance(), 0.0);
    }

    #[test]
    fn regression_passes_at_the_cutoff() {
        let decision = evaluate_regression(91, 100, 951, 1000, 0.05);
        assert_eq!(decision.verdict(), Verdict::Pass);
        assert_eq!(decision.cutoff(), 91);
        assert_eq!(
            evaluate_regression(90, 100, 951, 1000, 0.05).verdict(),
            Verdict::Fail
        );
    }

    #[test]
    fn envelopes_sum_alpha_by_direction() {
        let envelopes = type_one_envelopes([
            (DecisionRule::ComplianceExactBinomial, 0.01),
            (DecisionRule::RegressionFisher, 0.05),
            (DecisionRule::LatencyComplianceExactBinomial, 0.05),
        ]);
        assert_relative_eq!(envelopes.false_compliance().unwrap(), 0.06);
        assert_relative_eq!(envelopes.false_degradation_signal().unwrap(), 0.05);
        let none = type_one_envelopes([]);
        assert_eq!(none.false_compliance(), None);
        assert_eq!(none.false_degradation_signal(), None);
    }

    #[test]
    fn the_test_verdict_names_what_decided_it() {
        let overall = compose_overall_verdict(
            &pairs(&[("c1", Verdict::Pass), ("c2", Verdict::Fail)]),
            &pairs(&[("p95", Verdict::Fail)]),
        );
        assert_eq!(overall.verdict(), Verdict::Fail);
        let ids: Vec<(&str, TriggerKind)> = overall
            .triggering()
            .iter()
            .map(|t| (t.id(), t.kind()))
            .collect();
        assert_eq!(
            ids,
            [
                ("c2", TriggerKind::Criterion),
                ("p95", TriggerKind::Latency)
            ]
        );
    }

    #[test]
    fn a_latency_only_test_takes_the_latency_verdict() {
        let overall = compose_overall_verdict(&[], &pairs(&[("p95", Verdict::Pass)]));
        assert_eq!(overall.rate_verdict(), None);
        assert_eq!(overall.latency_verdict(), Some(Verdict::Pass));
        assert_eq!(overall.verdict(), Verdict::Pass);
        assert!(overall.triggering().is_empty());
    }

    #[test]
    fn functional_pass_and_latency_inconclusive_is_inconclusive() {
        let overall = compose_overall_verdict(
            &pairs(&[("c", Verdict::Pass)]),
            &pairs(&[("p99", Verdict::Inconclusive)]),
        );
        assert_eq!(overall.verdict(), Verdict::Inconclusive);
        assert_eq!(overall.triggering()[0].id(), "p99");
    }

    #[test]
    #[should_panic(expected = "a test verdict needs")]
    fn composing_nothing_is_a_defect() {
        let _ = compose_overall_verdict(&[], &[]);
    }
}
