//! Judgement: each criterion's tally becomes a decision under its rule.
//!
//! A declared requirement is decided by `compliance/exact-binomial`; a
//! baseline-derived criterion by `regression/fisher`, its cutoff derived at
//! the run's own size. The Wilson lower bound and the standard error are
//! descriptive context in both — they decide nothing. A zero-failures
//! criterion is observational and decided by no rule.

use crate::criteria::{CriteriaCounts, CriterionCounts, CriterionTarget};
use crate::model::ThresholdOrigin;
use crate::ptest::approach::{CriterionBaselineTally, SizingDesign};
use crate::statistics::decision::{evaluate_compliance, evaluate_regression};
use crate::statistics::regression::{
    MDD_POWER, design_power, minimum_detectable_degradation, resolved_power,
};
use crate::statistics::types::ConfidenceLevel;
use crate::verdict::{CriterionRow, DesignDisclosure, StatisticalAnalysis, Verdict};

/// What judging one criterion needs beyond its tally.
#[derive(Debug, Clone, Copy)]
pub(super) struct CriterionContext<'a> {
    /// The criterion's name.
    pub(super) name: &'a str,
    /// Its target.
    pub(super) target: &'a CriterionTarget,
    /// The confidence its decision is taken at.
    pub(super) confidence: ConfidenceLevel,
    /// Its baseline tally, for a baseline-derived criterion.
    pub(super) baseline: Option<&'a CriterionBaselineTally>,
    /// The origin a declared requirement is recorded with.
    pub(super) normative_origin: ThresholdOrigin,
    /// The design the run was sized for, when it was sized.
    pub(super) design: Option<SizingDesign>,
}

/// Builds one criterion's verdict row. A criterion with no in-scope trials
/// is `Inconclusive`; a zero-failures criterion is observational — `Pass`
/// iff it recorded no failures; otherwise its rule decides.
///
/// # Panics
///
/// Panics if a baseline-derived criterion has no baseline tally.
pub(super) fn criterion_row(
    context: &CriterionContext<'_>,
    counts: &CriteriaCounts,
) -> CriterionRow {
    let tally = counts.get(context.name);
    let pass = tally.map_or(0, CriterionCounts::pass);
    let fail = tally.map_or(0, CriterionCounts::fail);
    let distribution: Vec<(String, u32)> = tally.map_or_else(Vec::new, |t| {
        t.failure_distribution()
            .iter()
            .map(|(check, count)| (check.clone(), *count))
            .collect()
    });
    let total = pass + fail;
    if total == 0 {
        return CriterionRow::new(
            context.name,
            pass,
            fail,
            distribution,
            None,
            Verdict::Inconclusive,
        );
    }
    let (verdict, analysis) = match context.target {
        CriterionTarget::ZeroFailures => {
            let verdict = if fail == 0 {
                Verdict::Pass
            } else {
                Verdict::Fail
            };
            (verdict, None)
        }
        CriterionTarget::NormativeRate(rate) => {
            let (verdict, analysis) = judge_compliance(pass, total, *rate, context);
            (verdict, Some(analysis))
        }
        CriterionTarget::EmpiricalRate => {
            let baseline = context
                .baseline
                .expect("a baseline-derived criterion requires a baseline");
            let (verdict, analysis) = judge_regression(pass, total, baseline, context);
            (verdict, Some(analysis))
        }
    };
    CriterionRow::new(context.name, pass, fail, distribution, analysis, verdict)
}

/// Decides a declared requirement by `compliance/exact-binomial`.
fn judge_compliance(
    pass: u32,
    total: u32,
    requirement: f64,
    context: &CriterionContext<'_>,
) -> (Verdict, StatisticalAnalysis) {
    let decision = evaluate_compliance(pass, total, requirement, context.confidence.alpha());
    let analysis = StatisticalAnalysis::compliance(
        &decision,
        context.confidence.value(),
        context.normative_origin,
    );
    (decision.verdict(), analysis)
}

/// Decides a baseline-derived criterion by `regression/fisher`, and states
/// what the design can detect.
fn judge_regression(
    pass: u32,
    total: u32,
    baseline: &CriterionBaselineTally,
    context: &CriterionContext<'_>,
) -> (Verdict, StatisticalAnalysis) {
    let alpha = context.confidence.alpha();
    let decision = evaluate_regression(pass, total, baseline.successes, baseline.trials, alpha);
    let disclosure = design_disclosure(baseline, total, alpha, context.design);
    let analysis =
        StatisticalAnalysis::regression(&decision, context.confidence.value(), disclosure);
    (decision.verdict(), analysis)
}

/// What the regression design can detect: the minimum detectable
/// degradation always (the inversion of the design power at 80%), and the
/// design and resolved powers at the design alternative rate when the run
/// was sized for one.
fn design_disclosure(
    baseline: &CriterionBaselineTally,
    test_samples: u32,
    alpha: f64,
    design: Option<SizingDesign>,
) -> DesignDisclosure {
    let baseline_rate = baseline.rate();
    let minimum_detectable_degradation = minimum_detectable_degradation(
        baseline.trials,
        test_samples,
        alpha,
        baseline_rate,
        MDD_POWER,
    );
    let Some(design) = design else {
        return DesignDisclosure {
            minimum_detectable_degradation,
            ..DesignDisclosure::default()
        };
    };
    let rate = design.alternative.rate_for(baseline_rate);
    DesignDisclosure {
        minimum_detectable_degradation,
        design_alternative_rate: Some(rate),
        design_power: Some(design_power(
            baseline.trials,
            test_samples,
            alpha,
            baseline_rate,
            rate,
        )),
        resolved_test_power: Some(resolved_power(
            baseline.successes,
            baseline.trials,
            test_samples,
            alpha,
            rate,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::criteria::CriterionSampleResult;
    use crate::model::ContractViolation;
    use crate::ptest::approach::DesignAlternative;
    use crate::statistics::rules::DecisionRule;
    use crate::verdict::RuleEvidence;

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

    fn context<'a>(
        target: &'a CriterionTarget,
        baseline: Option<&'a CriterionBaselineTally>,
    ) -> CriterionContext<'a> {
        CriterionContext {
            name: "c",
            target,
            confidence: ConfidenceLevel::new(0.95),
            baseline,
            normative_origin: ThresholdOrigin::Sla,
            design: None,
        }
    }

    #[test]
    fn a_requirement_is_decided_by_the_exact_binomial_rule() {
        let target = CriterionTarget::NormativeRate(0.90);
        let row = criterion_row(&context(&target, None), &counts_of("c", 49, 1));
        assert_eq!(row.verdict(), Verdict::Pass);
        let analysis = row.statistical_analysis().unwrap();
        assert_eq!(
            analysis.decision_rule(),
            DecisionRule::ComplianceExactBinomial
        );
        assert_eq!(analysis.threshold_origin(), ThresholdOrigin::Sla);
        assert!((analysis.threshold() - 0.90).abs() < f64::EPSILON);
    }

    #[test]
    fn a_baseline_derived_criterion_is_decided_by_the_fisher_cutoff() {
        let target = CriterionTarget::EmpiricalRate;
        let baseline = CriterionBaselineTally {
            criterion_name: "c".to_owned(),
            successes: 951,
            trials: 1000,
        };
        let row = criterion_row(&context(&target, Some(&baseline)), &counts_of("c", 91, 9));
        assert_eq!(row.verdict(), Verdict::Pass);
        let analysis = row.statistical_analysis().unwrap();
        assert_eq!(analysis.decision_rule(), DecisionRule::RegressionFisher);
        assert!((analysis.threshold() - 0.91).abs() < 1e-12);
        let RuleEvidence::Regression(evidence) = analysis.evidence() else {
            panic!("a regression decision carries regression evidence");
        };
        assert_eq!(evidence.cutoff, 91);
        assert!(evidence.minimum_detectable_degradation.is_some());
        assert!(evidence.design_power.is_none());
    }

    #[test]
    fn a_sized_run_discloses_design_and_resolved_power() {
        let target = CriterionTarget::EmpiricalRate;
        let baseline = CriterionBaselineTally {
            criterion_name: "c".to_owned(),
            successes: 951,
            trials: 1000,
        };
        let mut sized = context(&target, Some(&baseline));
        sized.design = Some(SizingDesign {
            alternative: DesignAlternative::Absolute(0.925),
            target_power: 0.8,
        });
        let row = criterion_row(&sized, &counts_of("c", 950, 50));
        let RuleEvidence::Regression(evidence) = row.statistical_analysis().unwrap().evidence()
        else {
            panic!("a regression decision carries regression evidence");
        };
        assert_eq!(evidence.design_alternative_rate, Some(0.925));
        assert!(evidence.design_power.is_some());
        assert!(evidence.resolved_test_power.is_some());
    }

    #[test]
    fn a_zero_failures_criterion_is_observational() {
        let target = CriterionTarget::ZeroFailures;
        let row = criterion_row(&context(&target, None), &counts_of("c", 10, 0));
        assert_eq!(row.verdict(), Verdict::Pass);
        assert!(row.statistical_analysis().is_none());
    }

    #[test]
    fn a_criterion_with_no_trials_is_inconclusive() {
        let target = CriterionTarget::NormativeRate(0.9);
        let row = criterion_row(&context(&target, None), &CriteriaCounts::new());
        assert_eq!(row.verdict(), Verdict::Inconclusive);
    }
}
