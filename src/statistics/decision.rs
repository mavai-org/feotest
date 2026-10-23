//! Decisions: one criterion under its rule, and their composition
//! (Statistical Companion §1.4.6, §12.3.2).
//!
//! A criterion is decided by the rule its threshold's origin and the test's
//! intent select:
//!
//! - a **given** (normative) requirement under verification by
//!   `compliance/exact-binomial` — [`evaluate_compliance`];
//! - a given requirement under smoke by `compliance/smoke-exact-binomial` —
//!   [`evaluate_smoke_compliance`];
//! - a **baseline-derived** (empirical) bar by `regression/fisher` —
//!   [`evaluate_regression`].
//!
//! Criteria compose by one structural rule — PASS if every verdict passes,
//! FAIL if any fails, INCONCLUSIVE otherwise — and the same rule composes
//! the enforced dimensions among the functional dimension `V_rate` and the
//! latency dimension `V_latency` into the test's verdict `V_test` (§12.6):
//! an advisory dimension is decided and reported but never enters it. A
//! FAIL or an INCONCLUSIVE names what decided it. The Type-I envelopes are
//! disclosed by procedure direction over the binding decisions: the sum of
//! alpha over the compliance decisions (false compliance) and over the
//! regression decisions (false degradation signal).

use std::fmt;

use serde::{Serialize, Serializer};

use crate::statistics::compliance::{clopper_pearson_lower, minimum_passing_count};
use crate::statistics::distributions::binomial_upper_tail;
use crate::statistics::regression::{RegressionDerivation, derive_regression_cutoff};
use crate::statistics::rules::{DecisionRule, Direction, EnforcementMode};
use crate::statistics::smoke::{false_non_compliance, smoke_p_value, smoke_required_pass};

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
    p_value: f64,
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

    /// The p-value of the observed count, `P_{p_req}(K ≥ K_obs)`.
    #[must_use]
    pub const fn p_value(&self) -> f64 {
        self.p_value
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
        p_value: binomial_upper_tail(i64::from(successes), trials, requirement),
        false_compliance: minimum_passing.map_or(0.0, |k_min| {
            binomial_upper_tail(i64::from(k_min), trials, requirement)
        }),
        clopper_pearson_lower: clopper_pearson_lower(successes, trials, alpha),
    }
}

/// A criterion decided by `compliance/smoke-exact-binomial`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmokeComplianceDecision {
    verdict: Verdict,
    successes: u32,
    trials: u32,
    planned_samples: u32,
    requirement: f64,
    alpha: f64,
    required_pass: u32,
    minimum_passing: Option<u32>,
    p_value: f64,
    false_non_compliance: f64,
    clopper_pearson_lower: f64,
}

impl SmokeComplianceDecision {
    /// The rule that decided it.
    pub const RULE: DecisionRule = DecisionRule::ComplianceSmokeExactBinomial;

    /// PASS iff `K ≥ required_pass` — no evidence that the service falls
    /// short of its requirement, which is not evidence of compliance; FAIL
    /// once no outcome of the planned size can reach it — evidence of
    /// non-compliance at level alpha. INCONCLUSIVE only when a run stopped
    /// short of its plan before either was settled.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// `K`.
    #[must_use]
    pub const fn successes(&self) -> u32 {
        self.successes
    }

    /// The trials observed; fewer than planned only when the run stopped
    /// early.
    #[must_use]
    pub const fn trials(&self) -> u32 {
        self.trials
    }

    /// `n`, the size the rule was applied at: the planned sample count.
    #[must_use]
    pub const fn planned_samples(&self) -> u32 {
        self.planned_samples
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

    /// One more than the largest count that is evidence of non-compliance
    /// at the planned size; 0 when no count can fail there.
    #[must_use]
    pub const fn required_pass(&self) -> u32 {
        self.required_pass
    }

    /// Whether any outcome of the planned size could have failed.
    #[must_use]
    pub const fn fail_possible(&self) -> bool {
        self.required_pass > 0
    }

    /// `k_min` of `compliance/exact-binomial` at the planned size: the count
    /// a verification test of this size would need to demonstrate
    /// compliance; `None` when this size cannot demonstrate it at all.
    /// Reported; it decides nothing here.
    #[must_use]
    pub const fn minimum_passing(&self) -> Option<u32> {
        self.minimum_passing
    }

    /// Whether a verification test of this size could demonstrate
    /// compliance.
    #[must_use]
    pub const fn pass_possible(&self) -> bool {
        self.minimum_passing.is_some()
    }

    /// The p-value of the observed count, the lower tail
    /// `P_{p_req}(K ≤ K_obs)` over the observed trials.
    #[must_use]
    pub const fn p_value(&self) -> f64 {
        self.p_value
    }

    /// `P_{p_req}(K < required_pass)` at the planned size: the chance of a
    /// FAIL when the service exactly meets its requirement; 0 when no count
    /// can fail.
    #[must_use]
    pub const fn false_non_compliance(&self) -> f64 {
        self.false_non_compliance
    }

    /// The one-sided Clopper–Pearson lower bound at `1 − alpha`, reported
    /// beside the verdict; it decides nothing.
    #[must_use]
    pub const fn clopper_pearson_lower(&self) -> f64 {
        self.clopper_pearson_lower
    }
}

/// Decides a criterion against a given requirement under smoke intent.
///
/// The rule is applied at `planned_samples`, the size the run was planned
/// at, so a run that stopped early is decided by what its plan settled:
/// PASS once `successes` reaches the required count, FAIL once the samples
/// not run could no longer bring it there. A run of its full plan passes
/// `trials == planned_samples`.
///
/// # Panics
///
/// Panics if `trials` is zero or exceeds `planned_samples`, `successes`
/// exceeds `trials`, or the requirement or level is outside `(0, 1)`.
#[must_use]
pub fn evaluate_smoke_compliance(
    successes: u32,
    trials: u32,
    planned_samples: u32,
    requirement: f64,
    alpha: f64,
) -> SmokeComplianceDecision {
    assert!(successes <= trials, "successes must not exceed trials");
    assert!(
        trials <= planned_samples,
        "trials must not exceed the planned samples"
    );
    let required_pass = smoke_required_pass(requirement, planned_samples, alpha);
    let not_run = planned_samples - trials;
    let verdict = if successes >= required_pass {
        Verdict::Pass
    } else if successes + not_run < required_pass {
        Verdict::Fail
    } else {
        Verdict::Inconclusive
    };
    SmokeComplianceDecision {
        verdict,
        successes,
        trials,
        planned_samples,
        requirement,
        alpha,
        required_pass,
        minimum_passing: minimum_passing_count(requirement, planned_samples, alpha),
        p_value: smoke_p_value(successes, trials, requirement),
        false_non_compliance: false_non_compliance(required_pass, planned_samples, requirement),
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
// mavai-ref: JVI-T08PKZ6 — do not remove (resolves in mavai-orchestrator)
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

/// Sums each direction's alphas over the `(rule, alpha)` decisions made; a
/// decision whose rule enters neither envelope (a smoke rule) adds nothing.
#[must_use]
// mavai-ref: JVI-1PK1X9U — do not remove (resolves in mavai-orchestrator)
pub fn type_one_envelopes(decisions: impl IntoIterator<Item = (DecisionRule, f64)>) -> Envelopes {
    let mut envelopes = Envelopes {
        false_compliance: None,
        false_degradation_signal: None,
    };
    let directed = decisions
        .into_iter()
        .filter_map(|(rule, alpha)| rule.direction().map(|direction| (direction, alpha)));
    for (direction, alpha) in directed {
        let slot = match direction {
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
    /// A latency constraint.
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

/// One functional criterion or latency constraint, of an enforced
/// dimension, that decided the test.
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

/// One dimension's decisions as the test verdict composes them.
#[derive(Debug, Clone, Copy)]
pub struct DimensionDecisions<'a> {
    /// The `(id, verdict)` pair of each criterion or constraint; empty for a
    /// dimension the test does not carry.
    pub decisions: &'a [(String, Verdict)],
    /// Whether the run enforces the dimension or makes it advisory.
    pub mode: EnforcementMode,
}

impl DimensionDecisions<'_> {
    /// The dimension's composite verdict; `None` when the test does not
    /// carry it.
    fn verdict(&self) -> Option<Verdict> {
        structural_composite(self.decisions.iter().map(|(_, v)| *v))
    }

    /// The dimension's mode; `None` when the test does not carry it.
    fn present_mode(&self) -> Option<EnforcementMode> {
        (!self.decisions.is_empty()).then_some(self.mode)
    }

    /// The dimension's verdict when it binds the test.
    fn binding_verdict(&self) -> Option<Verdict> {
        self.verdict().filter(|_| self.mode.is_enforced())
    }
}

/// The test's verdict `V_test` and the two dimensions it composes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverallVerdict {
    rate_verdict: Option<Verdict>,
    latency_verdict: Option<Verdict>,
    functional_mode: Option<EnforcementMode>,
    latency_mode: Option<EnforcementMode>,
    verdict: Verdict,
    triggering: Vec<Trigger>,
}

impl OverallVerdict {
    /// `V_rate`, the composite of the functional criteria, enforced or
    /// advisory; `None` for a test with none.
    #[must_use]
    pub const fn rate_verdict(&self) -> Option<Verdict> {
        self.rate_verdict
    }

    /// `V_latency`, the composite of the latency constraints, enforced or
    /// advisory; `None` for a test with none.
    #[must_use]
    pub const fn latency_verdict(&self) -> Option<Verdict> {
        self.latency_verdict
    }

    /// The functional dimension's mode; `None` for a test with no criteria.
    #[must_use]
    pub const fn functional_mode(&self) -> Option<EnforcementMode> {
        self.functional_mode
    }

    /// The latency dimension's mode; `None` for a test with no latency
    /// constraint.
    #[must_use]
    pub const fn latency_mode(&self) -> Option<EnforcementMode> {
        self.latency_mode
    }

    /// `V_test`, the composite of the enforced dimensions; PASS when no
    /// dimension is enforced.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// For a FAIL or an INCONCLUSIVE, the criteria and constraints of the
    /// enforced dimensions whose verdict is the test's, criteria first.
    #[must_use]
    pub fn triggering(&self) -> &[Trigger] {
        &self.triggering
    }
}

/// Composes `V_test` over the enforced dimensions only (§12.6).
///
/// `V_test` is the structural composite of the enforced dimensions, PASS
/// when none is enforced. An advisory dimension keeps its verdict, reported
/// beside `V_test`, and never triggers it.
///
/// # Panics
///
/// Panics when the test carries neither a criterion nor a latency
/// constraint.
#[must_use]
pub fn compose_overall_verdict(
    functional: DimensionDecisions<'_>,
    latency: DimensionDecisions<'_>,
) -> OverallVerdict {
    assert!(
        !(functional.decisions.is_empty() && latency.decisions.is_empty()),
        "a test verdict needs a criterion or a latency constraint"
    );
    let verdict = structural_composite(
        functional
            .binding_verdict()
            .into_iter()
            .chain(latency.binding_verdict()),
    )
    .unwrap_or(Verdict::Pass);
    let triggering = if verdict == Verdict::Pass {
        Vec::new()
    } else {
        let named = |kind: TriggerKind, dimension: &DimensionDecisions<'_>| -> Vec<Trigger> {
            dimension
                .decisions
                .iter()
                .filter(|_| dimension.mode.is_enforced())
                .filter(|(_, v)| *v == verdict)
                .map(|(id, _)| Trigger {
                    kind,
                    id: id.clone(),
                })
                .collect()
        };
        let mut triggers = named(TriggerKind::Criterion, &functional);
        triggers.extend(named(TriggerKind::Latency, &latency));
        triggers
    };
    OverallVerdict {
        rate_verdict: functional.verdict(),
        latency_verdict: latency.verdict(),
        functional_mode: functional.present_mode(),
        latency_mode: latency.present_mode(),
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
    fn smoke_passes_an_undersized_design_without_evidence_of_shortfall() {
        // 0.99 over 100 samples: verification needs 299; smoke requires 97.
        let decision = evaluate_smoke_compliance(100, 100, 100, 0.99, 0.05);
        assert_eq!(decision.verdict(), Verdict::Pass);
        assert_eq!(decision.required_pass(), 97);
        assert!(!decision.pass_possible());
        assert_relative_eq!(decision.p_value(), 1.0);
        let short = evaluate_smoke_compliance(96, 100, 100, 0.99, 0.05);
        assert_eq!(short.verdict(), Verdict::Fail);
        assert_relative_eq!(
            short.p_value(),
            0.018_374_036_444_649_7,
            max_relative = 1e-10
        );
    }

    #[test]
    fn smoke_decides_a_run_stopped_early_by_its_plan() {
        // 97 of 97 run: success is settled at the planned size.
        let guaranteed = evaluate_smoke_compliance(97, 97, 100, 0.99, 0.05);
        assert_eq!(guaranteed.verdict(), Verdict::Pass);
        assert_eq!(guaranteed.required_pass(), 97);
        // The fourth miss at sample 30: no outcome can reach 97.
        let inevitable = evaluate_smoke_compliance(26, 30, 100, 0.99, 0.05);
        assert_eq!(inevitable.verdict(), Verdict::Fail);
        // Neither settled: a run cut short by its budget.
        let unsettled = evaluate_smoke_compliance(28, 30, 100, 0.99, 0.05);
        assert_eq!(unsettled.verdict(), Verdict::Inconclusive);
    }

    #[test]
    fn smoke_cannot_fail_where_no_count_is_evidence() {
        let decision = evaluate_smoke_compliance(0, 1, 1, 0.5, 0.05);
        assert_eq!(decision.verdict(), Verdict::Pass);
        assert!(!decision.fail_possible());
        assert_relative_eq!(decision.false_non_compliance(), 0.0);
    }

    #[test]
    #[should_panic(expected = "trials must not exceed the planned samples")]
    fn smoke_rejects_more_trials_than_planned() {
        let _ = evaluate_smoke_compliance(5, 6, 5, 0.9, 0.05);
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
    fn smoke_decisions_enter_neither_envelope() {
        let smoke_only = type_one_envelopes([
            (DecisionRule::ComplianceSmokeExactBinomial, 0.05),
            (DecisionRule::LatencySmokeComplianceExactBinomial, 0.05),
        ]);
        assert_eq!(smoke_only.false_compliance(), None);
        assert_eq!(smoke_only.false_degradation_signal(), None);
        let mixed = type_one_envelopes([
            (DecisionRule::ComplianceSmokeExactBinomial, 0.05),
            (DecisionRule::RegressionFisher, 0.01),
        ]);
        assert_eq!(mixed.false_compliance(), None);
        assert_relative_eq!(mixed.false_degradation_signal().unwrap(), 0.01);
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

    const ENFORCED: EnforcementMode = EnforcementMode::Enforced;
    const ADVISORY: EnforcementMode = EnforcementMode::Advisory;

    fn dimension(decisions: &[(String, Verdict)], mode: EnforcementMode) -> DimensionDecisions<'_> {
        DimensionDecisions { decisions, mode }
    }

    #[test]
    fn the_test_verdict_names_what_decided_it() {
        let criteria = pairs(&[("c1", Verdict::Pass), ("c2", Verdict::Fail)]);
        let latency = pairs(&[("p95", Verdict::Fail)]);
        let overall = compose_overall_verdict(
            dimension(&criteria, ENFORCED),
            dimension(&latency, ENFORCED),
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
        let latency = pairs(&[("p95", Verdict::Pass)]);
        let overall =
            compose_overall_verdict(dimension(&[], ENFORCED), dimension(&latency, ENFORCED));
        assert_eq!(overall.rate_verdict(), None);
        assert_eq!(overall.functional_mode(), None);
        assert_eq!(overall.latency_verdict(), Some(Verdict::Pass));
        assert_eq!(overall.latency_mode(), Some(ENFORCED));
        assert_eq!(overall.verdict(), Verdict::Pass);
        assert_eq!(overall.triggering(), []);
    }

    #[test]
    fn functional_pass_and_latency_inconclusive_is_inconclusive() {
        let criteria = pairs(&[("c", Verdict::Pass)]);
        let latency = pairs(&[("p99", Verdict::Inconclusive)]);
        let overall = compose_overall_verdict(
            dimension(&criteria, ENFORCED),
            dimension(&latency, ENFORCED),
        );
        assert_eq!(overall.verdict(), Verdict::Inconclusive);
        assert_eq!(overall.triggering()[0].id(), "p99");
    }

    #[test]
    fn an_advisory_dimension_is_reported_and_never_enters_the_verdict() {
        let criteria = pairs(&[("c", Verdict::Pass)]);
        let latency = pairs(&[("p95", Verdict::Fail)]);
        let overall = compose_overall_verdict(
            dimension(&criteria, ENFORCED),
            dimension(&latency, ADVISORY),
        );
        assert_eq!(overall.verdict(), Verdict::Pass);
        assert_eq!(overall.latency_verdict(), Some(Verdict::Fail));
        assert_eq!(overall.latency_mode(), Some(ADVISORY));
        assert_eq!(overall.triggering(), []);
    }

    #[test]
    fn an_advisory_dimension_never_triggers_the_enforced_one() {
        let criteria = pairs(&[("c", Verdict::Fail)]);
        let latency = pairs(&[("p95", Verdict::Fail)]);
        let overall = compose_overall_verdict(
            dimension(&criteria, ADVISORY),
            dimension(&latency, ENFORCED),
        );
        assert_eq!(overall.verdict(), Verdict::Fail);
        assert_eq!(overall.rate_verdict(), Some(Verdict::Fail));
        let ids: Vec<&str> = overall.triggering().iter().map(Trigger::id).collect();
        assert_eq!(ids, ["p95"]);
    }

    #[test]
    fn with_no_enforced_dimension_the_test_passes() {
        let criteria = pairs(&[("c", Verdict::Fail)]);
        let latency = pairs(&[("p95", Verdict::Inconclusive)]);
        let overall = compose_overall_verdict(
            dimension(&criteria, ADVISORY),
            dimension(&latency, ADVISORY),
        );
        assert_eq!(overall.verdict(), Verdict::Pass);
        assert_eq!(overall.triggering(), []);
        let advisory_only =
            compose_overall_verdict(dimension(&criteria, ADVISORY), dimension(&[], ENFORCED));
        assert_eq!(advisory_only.verdict(), Verdict::Pass);
        assert_eq!(advisory_only.latency_mode(), None);
    }

    #[test]
    #[should_panic(expected = "a test verdict needs")]
    fn composing_nothing_is_a_defect() {
        let _ = compose_overall_verdict(dimension(&[], ENFORCED), dimension(&[], ENFORCED));
    }
}
