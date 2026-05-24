//! The composite, per-criterion functional assessment.
//!
//! Where [`FunctionalDimension`](crate::verdict::FunctionalDimension) reports
//! the functional dimension as a single aggregate, a `FunctionalAssessment`
//! partitions it per criterion — one [`CriterionRow`] each — plus the
//! composite verdict over them. It is carried alongside the aggregate while
//! the spine is re-modelled; the single-criterion case populates exactly one
//! row whose verdict is the composite.

use serde::Serialize;

use crate::verdict::StatisticalAnalysis;
use crate::verdict::Verdict;

/// One criterion's line in the composite assessment.
///
/// Carries its name, its pass/fail tally (denominator `pass + fail`), the
/// statistical analysis behind its verdict (absent for observational
/// criteria), and its three-valued verdict.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CriterionRow {
    name: String,
    pass: u32,
    fail: u32,
    pass_rate: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    statistical_analysis: Option<StatisticalAnalysis>,
    verdict: Verdict,
}

impl CriterionRow {
    /// Builds a criterion row. The pass rate is derived from the tally
    /// (`pass / (pass + fail)`, or `0.0` when no trials were counted).
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        pass: u32,
        fail: u32,
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
            pass,
            fail,
            pass_rate,
            statistical_analysis,
            verdict,
        }
    }

    /// The criterion name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
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
/// The composite is the authoritative functional verdict. With one criterion
/// it equals that criterion's verdict; with several it is their conjunction,
/// becoming `Inconclusive` if any contributing row is.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionalAssessment {
    composite: Verdict,
    criteria: Vec<CriterionRow>,
}

impl FunctionalAssessment {
    /// Builds an assessment from an explicit composite verdict and its rows.
    #[must_use]
    pub const fn new(composite: Verdict, criteria: Vec<CriterionRow>) -> Self {
        Self {
            composite,
            criteria,
        }
    }

    /// Builds a single-criterion assessment — the composite is that row's
    /// verdict (composite-over-one).
    #[must_use]
    pub fn single(row: CriterionRow) -> Self {
        Self {
            composite: row.verdict(),
            criteria: vec![row],
        }
    }

    /// The composite verdict over the criteria.
    #[must_use]
    pub const fn composite(&self) -> Verdict {
        self.composite
    }

    /// The per-criterion rows, in declaration order.
    #[must_use]
    pub fn criteria(&self) -> &[CriterionRow] {
        &self.criteria
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_derives_pass_rate_and_total() {
        let row = CriterionRow::new("c", 8, 2, None, Verdict::Pass);
        assert_eq!(row.total(), 10);
        assert!((row.pass_rate() - 0.8).abs() < 1e-12);
    }

    #[test]
    fn row_with_no_trials_has_zero_pass_rate() {
        let row = CriterionRow::new("c", 0, 0, None, Verdict::Inconclusive);
        assert_eq!(row.total(), 0);
        assert!(row.pass_rate().abs() < 1e-12);
    }

    #[test]
    fn single_takes_its_composite_from_the_row() {
        let assessment =
            FunctionalAssessment::single(CriterionRow::new("c", 5, 5, None, Verdict::Fail));
        assert_eq!(assessment.composite(), Verdict::Fail);
        assert_eq!(assessment.criteria().len(), 1);
    }

    #[test]
    fn new_keeps_the_explicit_composite() {
        let assessment = FunctionalAssessment::new(
            Verdict::Inconclusive,
            vec![
                CriterionRow::new("a", 10, 0, None, Verdict::Pass),
                CriterionRow::new("b", 0, 0, None, Verdict::Inconclusive),
            ],
        );
        assert_eq!(assessment.composite(), Verdict::Inconclusive);
        assert_eq!(assessment.criteria().len(), 2);
    }
}
