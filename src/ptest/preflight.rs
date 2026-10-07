//! Preflight: is the configuration valid before any sample runs?
//!
//! Two configurations are refused before any invocation (Statistical
//! Companion §5.7.1), and a refusal names every applicable code, in one
//! fixed order, so a developer can correct every part at once:
//!
//! - `TEST_LARGER_THAN_BASELINE` — the test is planned larger than the
//!   baseline run it consumes; judged once on the two samplings, for every
//!   baseline-derived criterion and every baseline-derived latency
//!   constraint, whatever the intent;
//! - `COMPLIANCE_INFEASIBLE` — under verification intent, a normative design
//!   (a pass-rate requirement, or an explicit latency requirement) too small
//!   for any outcome to demonstrate compliance.
//!
//! When any part is invalid the whole configuration is refused: a run never
//! proceeds half-valid. A refusal applies whether the dimension it concerns
//! is enforced or advisory (§12.6). A normative test has no upper size
//! limit.

use crate::criteria::CriterionTarget;
use crate::latency::{ConstraintSource, ResolvedLatencyConstraint};
use crate::model::TestIntent;
use crate::ptest::approach::CriterionBaselineTally;
use crate::statistics::feasibility::feasibility_check;
use crate::statistics::rules::{ConfigurationError, check_test_size};
use crate::statistics::types::FeasibilityResult;

/// One invalid part of a configuration: its code, what it concerns, and the
/// figures that make it invalid.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RefusedPart {
    code: ConfigurationError,
    subject: String,
    detail: String,
}

impl RefusedPart {
    /// The configuration error.
    pub(super) const fn code(&self) -> ConfigurationError {
        self.code
    }

    /// What the part concerns and why it is invalid, in one line.
    pub(super) fn describe(&self) -> String {
        format!("{}: {}", self.subject, self.detail)
    }
}

/// A normative requirement the run must be able to demonstrate.
#[derive(Debug, Clone)]
pub(super) struct Requirement {
    /// The criterion's name, or the latency constraint's label.
    pub(super) subject: String,
    /// The required rate (a pass rate, or a latency percentile's share).
    pub(super) rate: f64,
    /// The decision's one-sided level.
    pub(super) alpha: f64,
}

/// The configuration facts the preflight judges.
#[derive(Debug, Clone, Copy)]
pub(super) struct Configuration<'a> {
    /// The planned sample count.
    pub(super) planned_samples: u32,
    /// Verification or smoke.
    pub(super) intent: TestIntent,
    /// Every baseline-derived criterion's baseline tally.
    pub(super) criterion_baselines: &'a [CriterionBaselineTally],
    /// The number of samples the baseline run executed, when a baseline
    /// resolved (the sampling a baseline-derived latency threshold consumes).
    pub(super) baseline_samples: Option<u32>,
    /// The resolved latency constraints.
    pub(super) latency: &'a [ResolvedLatencyConstraint],
}

/// Every invalid part of a test's configuration, in the fixed code order
/// (declaration order within a code).
pub(super) fn refused_parts(
    configuration: &Configuration<'_>,
    requirements: &[Requirement],
) -> Vec<RefusedPart> {
    let mut parts = larger_than_baseline(configuration);
    if configuration.intent == TestIntent::Verification {
        parts.extend(
            infeasible_requirements(configuration.planned_samples, requirements)
                .into_iter()
                .map(|(requirement, check)| infeasible_part(requirement, &check)),
        );
    }
    parts.sort_by_key(RefusedPart::code);
    parts
}

/// The `TEST_LARGER_THAN_BASELINE` parts: every baseline-derived criterion
/// and baseline-derived latency constraint whose baseline is smaller than
/// the planned test, whether its dimension is enforced or advisory.
fn larger_than_baseline(configuration: &Configuration<'_>) -> Vec<RefusedPart> {
    let planned = configuration.planned_samples;
    let criteria = configuration
        .criterion_baselines
        .iter()
        .map(|tally| (tally.criterion_name.clone(), tally.trials));
    let latency = configuration
        .baseline_samples
        .into_iter()
        .flat_map(|samples| {
            configuration
                .latency
                .iter()
                .filter(|c| c.is_baseline_derived())
                .map(move |c| (format!("latency {}", c.percentile()), samples))
        });
    criteria
        .chain(latency)
        .filter(|(_, baseline)| check_test_size(*baseline, planned).is_some())
        .map(|(subject, baseline)| RefusedPart {
            code: ConfigurationError::TestLargerThanBaseline,
            subject,
            detail: format!(
                "the test ({planned} samples) is larger than its baseline ({baseline} trials)"
            ),
        })
        .collect()
}

/// The requirements no outcome of the planned size can demonstrate, with
/// their feasibility checks.
pub(super) fn infeasible_requirements(
    planned_samples: u32,
    requirements: &[Requirement],
) -> Vec<(&Requirement, FeasibilityResult)> {
    requirements
        .iter()
        .map(|r| (r, feasibility_check(planned_samples, r.rate, r.alpha)))
        .filter(|(_, check)| !check.feasible())
        .collect()
}

/// The `COMPLIANCE_INFEASIBLE` part of one requirement.
fn infeasible_part(requirement: &Requirement, check: &FeasibilityResult) -> RefusedPart {
    RefusedPart {
        code: ConfigurationError::ComplianceInfeasible,
        subject: requirement.subject.clone(),
        detail: format!(
            "no count of {} can demonstrate {} at alpha {} (feasibility minimum {})",
            check.configured_samples(),
            check.target(),
            check.configured_alpha(),
            check.minimum_samples()
        ),
    }
}

/// The normative requirements of a configuration: each declared pass rate,
/// at its criterion's level, and each explicit latency ceiling, at the
/// latency criterion's level — whether its dimension is enforced or
/// advisory, since a design no outcome could decide is refused whatever
/// binds.
pub(super) fn requirements(
    targets: &[(&str, &CriterionTarget)],
    criterion_alpha: impl Fn(&str) -> f64,
    latency: &[ResolvedLatencyConstraint],
) -> Vec<Requirement> {
    let pass_rates = targets.iter().filter_map(|(name, target)| match target {
        CriterionTarget::NormativeRate(rate) => Some(Requirement {
            subject: (*name).to_owned(),
            rate: *rate,
            alpha: criterion_alpha(name),
        }),
        CriterionTarget::EmpiricalRate | CriterionTarget::ZeroFailures => None,
    });
    let ceilings = latency
        .iter()
        .filter(|c| matches!(c.source(), ConstraintSource::Explicit { .. }))
        .map(|c| Requirement {
            subject: format!("latency {}", c.percentile()),
            rate: c.percentile().as_fraction(),
            alpha: crate::statistics::rules::alpha_from_confidence(c.confidence()),
        });
    pass_rates.chain(ceilings).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(name: &str, trials: u32) -> CriterionBaselineTally {
        CriterionBaselineTally {
            criterion_name: name.to_owned(),
            successes: trials,
            trials,
        }
    }

    const fn configuration(
        planned: u32,
        intent: TestIntent,
        baselines: &[CriterionBaselineTally],
    ) -> Configuration<'_> {
        Configuration {
            planned_samples: planned,
            intent,
            criterion_baselines: baselines,
            baseline_samples: None,
            latency: &[],
        }
    }

    fn strict_requirement() -> Requirement {
        Requirement {
            subject: "compliance".to_owned(),
            rate: 0.999,
            alpha: 0.05,
        }
    }

    #[test]
    fn a_valid_configuration_has_no_refused_part() {
        let baselines = [tally("regression", 1000)];
        let parts = refused_parts(
            &configuration(100, TestIntent::Verification, &baselines),
            &[],
        );
        assert_eq!(parts.len(), 0);
    }

    #[test]
    fn every_invalid_part_is_reported_in_the_fixed_order() {
        let baselines = [tally("regression", 100)];
        let parts = refused_parts(
            &configuration(200, TestIntent::Verification, &baselines),
            &[strict_requirement()],
        );
        let codes: Vec<ConfigurationError> = parts.iter().map(RefusedPart::code).collect();
        assert_eq!(
            codes,
            [
                ConfigurationError::TestLargerThanBaseline,
                ConfigurationError::ComplianceInfeasible
            ]
        );
        assert!(parts[1].describe().contains("feasibility minimum 2995"));
    }

    #[test]
    fn smoke_intent_runs_an_infeasible_requirement() {
        let parts = refused_parts(
            &configuration(200, TestIntent::Smoke, &[]),
            &[strict_requirement()],
        );
        assert_eq!(parts.len(), 0);
    }

    #[test]
    fn a_normative_test_has_no_upper_size_limit() {
        let requirement = Requirement {
            subject: "compliance".to_owned(),
            rate: 0.9,
            alpha: 0.05,
        };
        let parts = refused_parts(
            &configuration(100_000, TestIntent::Verification, &[]),
            &[requirement],
        );
        assert_eq!(parts.len(), 0);
    }
}
