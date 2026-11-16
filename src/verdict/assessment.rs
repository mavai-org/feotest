//! The composite, per-criterion functional assessment.
//!
//! A `FunctionalAssessment` partitions a verdict's functional dimension per
//! criterion — one [`CriterionRow`] each — plus the composite verdict over
//! them. It is the verdict record's functional block; the single-criterion
//! case populates exactly one row whose verdict is the composite.

use serde::Serialize;
use serde::ser::SerializeMap;

use crate::statistics::decision::structural_composite;
use crate::statistics::rules::EnforcementMode;
use crate::verdict::StatisticalAnalysis;
use crate::verdict::Verdict;

/// A criterion's mode (Statistical Companion §1.4.5).
///
/// An inferential criterion estimates its pass rate and is decided by a
/// rule. An observational criterion is a zero-failures commitment, decided
/// by observation alone: PASS iff no trial failed, FAIL on any failed
/// trial, INCONCLUSIVE only with no trials. It estimates no rate and
/// carries no threshold, so nothing reports it as either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CriterionMode {
    /// Decided by a rule against a rate (the default).
    #[default]
    Inferential,
    /// A zero-failures criterion, decided by observation alone.
    Observational,
}

impl CriterionMode {
    /// The mode's wire name, as the verdict interchange format spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Inferential => "inferential",
            Self::Observational => "observational",
        }
    }

    /// Whether the mode is observational.
    #[must_use]
    pub const fn is_observational(self) -> bool {
        matches!(self, Self::Observational)
    }

    /// Whether the mode is inferential (for serde's `skip_serializing_if`).
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    const fn is_inferential(&self) -> bool {
        matches!(self, Self::Inferential)
    }
}

/// One criterion's line in the composite assessment.
///
/// Carries its name, its mode, its pass/fail tally (denominator
/// `pass + fail`), the statistical analysis behind its verdict (absent for
/// observational criteria), and its three-valued verdict.
// mavai-ref: JVI-8E4WNW5 — do not remove (resolves in mavai-orchestrator)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CriterionRow {
    name: String,
    #[serde(skip_serializing_if = "CriterionMode::is_inferential")]
    mode: CriterionMode,
    pass: u32,
    fail: u32,
    pass_rate: f64,
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_failure_distribution"
    )]
    failure_distribution: Vec<(String, u32)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    statistical_analysis: Option<StatisticalAnalysis>,
    verdict: Verdict,
}

/// Serialises a failure distribution (check name → count) as a JSON object.
fn serialize_failure_distribution<S: serde::Serializer>(
    pairs: &[(String, u32)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(Some(pairs.len()))?;
    for (check, count) in pairs {
        map.serialize_entry(check, count)?;
    }
    map.end()
}

impl CriterionRow {
    /// Builds a criterion row. The pass rate is derived from the tally
    /// (`pass / (pass + fail)`, or `0.0` when no trials were counted).
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        pass: u32,
        fail: u32,
        failure_distribution: Vec<(String, u32)>,
        statistical_analysis: Option<StatisticalAnalysis>,
        verdict: Verdict,
    ) -> Self {
        let total = pass + fail;
        let pass_rate = if total > 0 {
            f64::from(pass) / f64::from(total)
        } else {
            0.0
        };
        Self {
            name: name.into(),
            mode: CriterionMode::Inferential,
            pass,
            fail,
            pass_rate,
            failure_distribution,
            statistical_analysis,
            verdict,
        }
    }

    /// Convenience for the conventional single criterion of a non-decomposed
    /// run — named `"result"`, with no separate statistical analysis on the row.
    #[must_use]
    pub fn result(
        pass: u32,
        fail: u32,
        failure_distribution: Vec<(String, u32)>,
        verdict: Verdict,
    ) -> Self {
        Self::new("result", pass, fail, failure_distribution, None, verdict)
    }

    /// The same row for an observational (zero-failures) criterion. It
    /// carries no statistical analysis: observation alone decided it.
    ///
    /// # Panics
    ///
    /// Panics if the row carries a statistical analysis.
    #[must_use]
    pub fn observational(mut self) -> Self {
        assert!(
            self.statistical_analysis.is_none(),
            "an observational criterion is decided by no rule"
        );
        self.mode = CriterionMode::Observational;
        self
    }

    /// The criterion name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The criterion's mode: inferential unless it is a zero-failures
    /// criterion.
    #[must_use]
    pub const fn mode(&self) -> CriterionMode {
        self.mode
    }

    /// Clean passes.
    #[must_use]
    pub const fn pass(&self) -> u32 {
        self.pass
    }

    /// Failures (failing postcondition or failed transform).
    #[must_use]
    pub const fn fail(&self) -> u32 {
        self.fail
    }

    /// The denominator: every in-scope trial (`pass + fail`).
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.pass + self.fail
    }

    /// The observed pass rate.
    #[must_use]
    pub const fn pass_rate(&self) -> f64 {
        self.pass_rate
    }

    /// The failure distribution: counts keyed by the failing check's name.
    #[must_use]
    pub fn failure_distribution(&self) -> &[(String, u32)] {
        &self.failure_distribution
    }

    /// What an observational row observed, in words — its counts alone,
    /// never a rate: `"no failure in 300 trials"`, `"11 failures in 3000
    /// trials"`, or `"no trials"`.
    #[must_use]
    pub(crate) fn observation(&self) -> String {
        let total = self.total();
        match self.fail {
            _ if total == 0 => "no trials".to_owned(),
            0 => format!("no failure in {total} trials"),
            1 => format!("1 failure in {total} trials"),
            failures => format!("{failures} failures in {total} trials"),
        }
    }

    /// The statistical analysis behind the verdict, if inferential.
    #[must_use]
    pub const fn statistical_analysis(&self) -> Option<&StatisticalAnalysis> {
        self.statistical_analysis.as_ref()
    }

    /// The criterion's three-valued verdict.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }
}

/// The composite functional assessment: the per-criterion rows and the
/// composite verdict over them.
///
/// The composite is the functional dimension's verdict `V_rate`, by the
/// structural rule of Statistical Companion §1.4.6: PASS if every criterion
/// passes, FAIL if any fails, INCONCLUSIVE otherwise. With one criterion it
/// equals that criterion's verdict. A refused configuration judged nothing
/// and has no rows and no composite.
///
/// The assessment carries the run's mode for the functional dimension
/// (§12.6): enforced, the default, when `V_rate` enters the test verdict;
/// advisory when the run reports it beside the test verdict instead. The
/// criteria are decided by their rules either way.
// mavai-ref: JVI-60WEAWK — do not remove (resolves in mavai-orchestrator)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionalAssessment {
    #[serde(skip_serializing_if = "Option::is_none")]
    composite: Option<Verdict>,
    mode: EnforcementMode,
    criteria: Vec<CriterionRow>,
}

impl FunctionalAssessment {
    /// Builds an enforced assessment from its rows; the composite is their
    /// structural composite.
    ///
    /// # Panics
    ///
    /// Panics if `criteria` is empty — a decided test judges at least one
    /// criterion.
    #[must_use]
    pub fn new(criteria: Vec<CriterionRow>) -> Self {
        let composite = structural_composite(criteria.iter().map(CriterionRow::verdict));
        assert!(
            composite.is_some(),
            "a functional assessment needs at least one criterion row"
        );
        Self {
            composite,
            mode: EnforcementMode::Enforced,
            criteria,
        }
    }

    /// The same assessment under the run's mode for the functional
    /// dimension.
    #[must_use]
    pub const fn with_mode(mut self, mode: EnforcementMode) -> Self {
        self.mode = mode;
        self
    }

    /// Builds a single-criterion assessment — the composite is that row's
    /// verdict (composite-over-one).
    #[must_use]
    pub fn single(row: CriterionRow) -> Self {
        Self::new(vec![row])
    }

    /// The assessment of a refused configuration: no rows, no composite.
    #[must_use]
    pub(crate) const fn refused() -> Self {
        Self {
            composite: None,
            mode: EnforcementMode::Enforced,
            criteria: Vec::new(),
        }
    }

    /// The composite verdict over the criteria, `V_rate`.
    ///
    /// # Panics
    ///
    /// Panics on the assessment of a refused configuration, which judged
    /// nothing — check [`VerdictRecord::is_refused`](crate::verdict::VerdictRecord::is_refused)
    /// first.
    #[must_use]
    pub const fn composite(&self) -> Verdict {
        self.composite
            .expect("a refused configuration has no functional composite")
    }

    /// Whether the functional dimension binds the test (enforced) or is
    /// reported only (advisory).
    #[must_use]
    pub const fn mode(&self) -> EnforcementMode {
        self.mode
    }

    /// The per-criterion rows, in declaration order.
    #[must_use]
    pub fn criteria(&self) -> &[CriterionRow] {
        &self.criteria
    }

    /// Whether every criterion is observational: the test then makes no
    /// inferential claim, so it carries no statistics and no sizing
    /// approach. `false` for a refused configuration, which judged nothing.
    #[must_use]
    pub fn is_observational(&self) -> bool {
        !self.criteria.is_empty()
            && self
                .criteria
                .iter()
                .all(|row| row.mode().is_observational())
    }

    /// The row that summarises the functional dimension: the first
    /// criterion decided by a rule, else the first inferential criterion,
    /// else the first criterion. An observational criterion summarises
    /// the dimension only when no criterion is inferential.
    #[must_use]
    pub(crate) fn summary_row(&self) -> Option<&CriterionRow> {
        self.criteria
            .iter()
            .find(|row| row.statistical_analysis().is_some())
            .or_else(|| {
                self.criteria
                    .iter()
                    .find(|row| !row.mode().is_observational())
            })
            .or_else(|| self.criteria.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_derives_pass_rate_and_total() {
        let row = CriterionRow::new("c", 8, 2, vec![], None, Verdict::Pass);
        assert_eq!(row.total(), 10);
        assert!((row.pass_rate() - 0.8).abs() < 1e-12);
    }

    #[test]
    fn row_with_no_trials_has_zero_pass_rate() {
        let row = CriterionRow::new("c", 0, 0, vec![], None, Verdict::Inconclusive);
        assert_eq!(row.total(), 0);
        assert!(row.pass_rate().abs() < 1e-12);
    }

    #[test]
    fn result_row_is_named_and_carries_its_distribution() {
        let row = CriterionRow::result(7, 3, vec![("parse".to_string(), 3)], Verdict::Fail);
        assert_eq!(row.name(), "result");
        assert!(row.statistical_analysis().is_none());
        assert_eq!(row.failure_distribution(), [("parse".to_string(), 3)]);
    }

    #[test]
    fn single_takes_its_composite_from_the_row() {
        let assessment =
            FunctionalAssessment::single(CriterionRow::new("c", 5, 5, vec![], None, Verdict::Fail));
        assert_eq!(assessment.composite(), Verdict::Fail);
        assert_eq!(assessment.criteria().len(), 1);
    }

    #[test]
    fn composite_is_the_structural_composite_of_the_rows() {
        let inconclusive = FunctionalAssessment::new(vec![
            CriterionRow::new("a", 10, 0, vec![], None, Verdict::Pass),
            CriterionRow::new("b", 0, 0, vec![], None, Verdict::Inconclusive),
        ]);
        assert_eq!(inconclusive.composite(), Verdict::Inconclusive);
        assert_eq!(inconclusive.criteria().len(), 2);
        let failed = FunctionalAssessment::new(vec![
            CriterionRow::new("a", 5, 5, vec![], None, Verdict::Fail),
            CriterionRow::new("b", 0, 0, vec![], None, Verdict::Inconclusive),
        ]);
        assert_eq!(failed.composite(), Verdict::Fail);
    }

    #[test]
    fn an_assessment_is_enforced_unless_the_run_makes_it_advisory() {
        let row = CriterionRow::new("c", 5, 5, vec![], None, Verdict::Fail);
        let enforced = FunctionalAssessment::single(row.clone());
        assert_eq!(enforced.mode(), EnforcementMode::Enforced);
        let advisory = FunctionalAssessment::single(row).with_mode(EnforcementMode::Advisory);
        assert_eq!(advisory.mode(), EnforcementMode::Advisory);
        assert_eq!(advisory.composite(), Verdict::Fail);
    }

    #[test]
    fn a_refused_assessment_has_no_rows() {
        assert!(FunctionalAssessment::refused().criteria().is_empty());
        assert!(!FunctionalAssessment::refused().is_observational());
    }

    #[test]
    fn a_row_is_inferential_unless_marked_observational() {
        let row = CriterionRow::new("c", 5, 0, vec![], None, Verdict::Pass);
        assert_eq!(row.mode(), CriterionMode::Inferential);
        assert_eq!(row.observational().mode(), CriterionMode::Observational);
    }

    #[test]
    #[should_panic(expected = "decided by no rule")]
    fn a_row_decided_by_a_rule_cannot_be_observational() {
        let row = crate::oracle_examples::regression_row("worked_example_pass_at_cutoff", "c");
        let _ = row.observational();
    }

    #[test]
    fn an_observation_states_counts_never_a_rate() {
        let observation = |pass, fail| {
            CriterionRow::new("c", pass, fail, vec![], None, Verdict::Pass).observation()
        };
        assert_eq!(observation(300, 0), "no failure in 300 trials");
        assert_eq!(observation(999, 1), "1 failure in 1000 trials");
        assert_eq!(observation(2989, 11), "11 failures in 3000 trials");
        assert_eq!(observation(0, 0), "no trials");
    }

    #[test]
    fn an_assessment_is_observational_iff_every_row_is() {
        let zero = CriterionRow::new("zero", 10, 0, vec![], None, Verdict::Pass).observational();
        let rate = CriterionRow::new("rate", 10, 0, vec![], None, Verdict::Pass);
        assert!(FunctionalAssessment::single(zero.clone()).is_observational());
        assert!(!FunctionalAssessment::new(vec![zero, rate]).is_observational());
    }

    #[test]
    fn the_summary_row_is_the_first_decided_by_a_rule() {
        let zero = CriterionRow::new("zero", 10, 0, vec![], None, Verdict::Pass).observational();
        let undecided = CriterionRow::new("undecided", 0, 0, vec![], None, Verdict::Inconclusive);
        let decided =
            crate::oracle_examples::regression_row("worked_example_pass_at_cutoff", "decided");
        let mixed = FunctionalAssessment::new(vec![zero.clone(), undecided.clone(), decided]);
        assert_eq!(mixed.summary_row().unwrap().name(), "decided");
        let without_rule = FunctionalAssessment::new(vec![zero.clone(), undecided]);
        assert_eq!(without_rule.summary_row().unwrap().name(), "undecided");
        let observational = FunctionalAssessment::single(zero);
        assert_eq!(observational.summary_row().unwrap().name(), "zero");
        assert!(FunctionalAssessment::refused().summary_row().is_none());
    }
}
