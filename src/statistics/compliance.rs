//! Normative compliance under `compliance/exact-binomial` (Statistical
//! Companion §3.6, §5.5, §5.7).
//!
//! A requirement `p_req` is demonstrated by the exact one-sided binomial test
//! of `H0: p ≤ p_req` against `H1: p > p_req`. The decision artefact is the
//! smallest passing count `k_min = min{k : P_{p_req}(K ≥ k) ≤ alpha}`; the
//! test passes iff `K ≥ k_min`. A pass means the evidence supports
//! compliance at the configured level; a fail means compliance was not
//! demonstrated — not that the rate is below the requirement.
//!
//! A design can pass at all only if the all-success outcome clears the
//! test, `p_req^n ≤ alpha` — the feasibility minimum
//! `⌈ln alpha / ln p_req⌉`. Compliance tests are sized for the chance of a
//! pass when the service is in fact better than required, at a design
//! alternative rate above `p_req`.

use crate::statistics::distributions::{beta_quantile, binomial_upper_tail};
use crate::statistics::exact::{at_most_alpha, binomial_upper_tail_exact};

/// How far the exact sizing search looks before declaring a design
/// unsettled.
pub const DEFAULT_SIZING_HORIZON: u32 = 20_000;

/// Validates a requirement and a level.
fn validate(requirement: f64, alpha: f64) {
    assert!(
        requirement > 0.0 && requirement < 1.0,
        "the requirement must be strictly between 0 and 1, got {requirement}"
    );
    assert!(
        alpha > 0.0 && alpha < 1.0,
        "alpha must be strictly between 0 and 1, got {alpha}"
    );
}

/// Whether a count demonstrates compliance at this size: its upper tail
/// under the requirement is at most alpha (exact-boundary convention).
fn admits(count: u32, samples: u32, requirement: f64, alpha: f64) -> bool {
    at_most_alpha(
        binomial_upper_tail(i64::from(count), samples, requirement),
        alpha,
        || binomial_upper_tail_exact(count, samples, requirement),
    )
}

/// `k_min` of `compliance/exact-binomial`; `None` when no count can pass.
///
/// # Panics
///
/// Panics if `samples` is zero, or the requirement or level is outside
/// `(0, 1)`.
#[must_use]
pub fn minimum_passing_count(requirement: f64, samples: u32, alpha: f64) -> Option<u32> {
    assert!(samples > 0, "samples must be positive");
    validate(requirement, alpha);
    if !admits(samples, samples, requirement, alpha) {
        return None;
    }
    // The upper tail falls as the count rises: bisect for the first admitted.
    let (mut low, mut high) = (0_u32, samples);
    while low < high {
        let mid = low + (high - low) / 2;
        if admits(mid, samples, requirement, alpha) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    Some(low)
}

/// `k_min` walked from a neighbouring size's value: consecutive sizes'
/// minimums differ by at most one, so a sizing scan walks rather than
/// bisects. `guess` of `None` bisects afresh.
fn minimum_passing_count_near(
    requirement: f64,
    samples: u32,
    alpha: f64,
    guess: Option<u32>,
) -> Option<u32> {
    let Some(guess) = guess else {
        return minimum_passing_count(requirement, samples, alpha);
    };
    let mut count = guess.min(samples);
    while count <= samples && !admits(count, samples, requirement, alpha) {
        count += 1;
    }
    if count > samples {
        return None;
    }
    while count > 0 && admits(count - 1, samples, requirement, alpha) {
        count -= 1;
    }
    Some(count)
}

/// The smallest size at which a pass is possible, `⌈ln alpha / ln p_req⌉`.
///
/// Confirmed against the definition (`p_req^n ≤ alpha`, under the
/// exact-boundary convention) so floating point cannot move it by one.
///
/// # Panics
///
/// Panics if the requirement or level is outside `(0, 1)`.
#[must_use]
// mavai-ref: JVI-RDWGWVV — do not remove (resolves in mavai-orchestrator)
pub fn minimum_feasible_samples(requirement: f64, alpha: f64) -> u32 {
    validate(requirement, alpha);
    let feasible = |samples: u32| admits(samples, samples, requirement, alpha);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "both logarithms are negative, so the ratio is positive; realistic minimums fit in u32"
    )]
    let mut samples = (alpha.log(requirement).ceil() as u32).max(1);
    while samples > 1 && feasible(samples - 1) {
        samples -= 1;
    }
    while !feasible(samples) {
        samples += 1;
    }
    samples
}

/// The one-sided Clopper–Pearson lower bound at level `1 − alpha`.
///
/// Reported beside a compliance verdict; it decides nothing. Zero at no
/// successes.
///
/// # Panics
///
/// Panics if `samples` is zero, `successes` exceeds it, or `alpha` is not in
/// `(0, 1)`.
#[must_use]
pub fn clopper_pearson_lower(successes: u32, samples: u32, alpha: f64) -> f64 {
    assert!(samples > 0, "samples must be positive");
    assert!(successes <= samples, "successes must not exceed samples");
    assert!(
        alpha > 0.0 && alpha < 1.0,
        "alpha must be strictly between 0 and 1, got {alpha}"
    );
    if successes == 0 {
        return 0.0;
    }
    beta_quantile(
        alpha,
        f64::from(successes),
        f64::from(samples - successes + 1),
    )
}

/// `P(PASS)` of the design at a true rate: `P_rate(K ≥ k_min)`, 0 when no
/// count can pass.
///
/// # Panics
///
/// Panics on the inputs [`minimum_passing_count`] rejects, or a rate outside
/// `[0, 1]`.
#[must_use]
pub fn pass_probability(requirement: f64, samples: u32, alpha: f64, rate: f64) -> f64 {
    minimum_passing_count(requirement, samples, alpha).map_or(0.0, |k_min| {
        binomial_upper_tail(i64::from(k_min), samples, rate)
    })
}

/// Where a compliance sizing alternative came from; the report names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlternativeKind {
    /// `p_req + δ`: the declared assurance margin above the requirement.
    Margin,
    /// `(p_req + 1) / 2`: the default where `p_req + δ ≥ 1` leaves the unit
    /// interval.
    Midway,
    /// The design alternative rate the contract declared directly.
    Declared,
}

impl AlternativeKind {
    /// The kind's name, as a report states it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Margin => "MARGIN",
            Self::Midway => "MIDWAY",
            Self::Declared => "DECLARED",
        }
    }
}

/// The design alternative rate a compliance design is sized at, and its
/// kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizingAlternative {
    rate: f64,
    kind: AlternativeKind,
}

impl SizingAlternative {
    /// The design alternative rate.
    #[must_use]
    pub const fn rate(&self) -> f64 {
        self.rate
    }

    /// Where the rate came from.
    #[must_use]
    pub const fn kind(&self) -> AlternativeKind {
        self.kind
    }
}

/// The declared alternative, else `p_req + δ`, else the midway rate.
///
/// # Panics
///
/// Panics if a declared rate is outside `(p_req, 1]`.
#[must_use]
pub fn compliance_sizing_alternative(
    requirement: f64,
    margin: f64,
    declared_rate: Option<f64>,
) -> SizingAlternative {
    if let Some(rate) = declared_rate {
        assert!(
            requirement < rate && rate <= 1.0,
            "a declared alternative rate must lie in (requirement, 1], got {rate}"
        );
        return SizingAlternative {
            rate,
            kind: AlternativeKind::Declared,
        };
    }
    if requirement + margin < 1.0 {
        SizingAlternative {
            rate: requirement + margin,
            kind: AlternativeKind::Margin,
        }
    } else {
        SizingAlternative {
            rate: f64::midpoint(requirement, 1.0),
            kind: AlternativeKind::Midway,
        }
    }
}

/// Exact sizing of a compliance design (§5.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComplianceSizing {
    required_samples: Option<u32>,
    first_crossing: Option<u32>,
    achieved_power: Option<f64>,
    alternative: SizingAlternative,
}

impl ComplianceSizing {
    /// The smallest `n` from which the power at the alternative stays at or
    /// above the target up to the horizon; `None` when the design has not
    /// settled by the horizon (it is expensive, not invalid).
    #[must_use]
    pub const fn required_samples(&self) -> Option<u32> {
        self.required_samples
    }

    /// The smallest `n` whose power first reaches the target — reported,
    /// never the answer; `None` if none does.
    #[must_use]
    pub const fn first_crossing(&self) -> Option<u32> {
        self.first_crossing
    }

    /// The power at [`required_samples`](Self::required_samples).
    #[must_use]
    pub const fn achieved_power(&self) -> Option<f64> {
        self.achieved_power
    }

    /// The design alternative rate used, and its kind.
    #[must_use]
    pub const fn alternative(&self) -> SizingAlternative {
        self.alternative
    }
}

/// The smallest size from which `P(PASS | alternative)` stays at target.
///
/// Power is a sawtooth in `n`, and zero wherever the design is infeasible,
/// so the feasibility gate sits inside the search.
///
/// # Panics
///
/// Panics if the requirement or level is outside `(0, 1)`, `horizon` is
/// zero, or a declared rate is outside `(p_req, 1]`.
#[must_use]
pub fn size_compliance(
    requirement: f64,
    margin: f64,
    alpha: f64,
    power: f64,
    declared_rate: Option<f64>,
    horizon: u32,
) -> ComplianceSizing {
    validate(requirement, alpha);
    assert!(horizon > 0, "the sizing horizon must be positive");
    let alternative = compliance_sizing_alternative(requirement, margin, declared_rate);
    let powers = powers_up_to(requirement, alpha, alternative.rate, horizon);
    let first_crossing = powers.iter().position(|&p| p >= power);
    let last_short = powers.iter().rposition(|&p| p < power);
    let horizon_index = powers.len() - 1;
    let settled = match last_short {
        Some(index) if index == horizon_index => None,
        Some(index) => Some(index + 1),
        None => Some(0),
    };
    ComplianceSizing {
        required_samples: settled.map(size_at),
        first_crossing: first_crossing.map(size_at),
        achieved_power: settled.map(|index| powers[index]),
        alternative,
    }
}

/// The power at the alternative for every size `1..=horizon`.
fn powers_up_to(requirement: f64, alpha: f64, alternative_rate: f64, horizon: u32) -> Vec<f64> {
    let mut powers = Vec::with_capacity(horizon as usize);
    let mut previous: Option<u32> = None;
    for samples in 1..=horizon {
        let k_min = minimum_passing_count_near(requirement, samples, alpha, previous);
        powers.push(k_min.map_or(0.0, |k| {
            binomial_upper_tail(i64::from(k), samples, alternative_rate)
        }));
        previous = k_min.or(previous);
    }
    powers
}

/// The sample size at a zero-based position of the size scan.
fn size_at(index: usize) -> u32 {
    u32::try_from(index + 1).expect("the scan never exceeds the u32 horizon")
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn minimum_passing_count_is_the_first_admitted_count() {
        let k_min = minimum_passing_count(0.95, 150, 0.05).unwrap();
        assert!(binomial_upper_tail(i64::from(k_min), 150, 0.95) <= 0.05);
        assert!(binomial_upper_tail(i64::from(k_min) - 1, 150, 0.95) > 0.05);
    }

    #[test]
    fn no_count_passes_an_infeasible_design() {
        // 0.95^50 > 0.05: not even 50 of 50 demonstrates 0.95.
        assert_eq!(minimum_passing_count(0.95, 50, 0.05), None);
    }

    #[test]
    fn feasibility_minimum_for_095_at_005_is_59() {
        assert_eq!(minimum_feasible_samples(0.95, 0.05), 59);
    }

    #[test]
    fn exact_boundary_feasibility_is_inclusive() {
        // 0.5^5 = 1/32 = 0.03125 exactly: n = 5 is feasible at that alpha.
        assert_eq!(minimum_feasible_samples(0.5, 0.031_25), 5);
        assert_eq!(minimum_passing_count(0.5, 5, 0.031_25), Some(5));
    }

    #[test]
    fn walked_minimums_agree_with_bisection() {
        let mut previous = None;
        for samples in 1..=200 {
            let walked = minimum_passing_count_near(0.9, samples, 0.05, previous);
            assert_eq!(walked, minimum_passing_count(0.9, samples, 0.05));
            previous = walked.or(previous);
        }
    }

    #[test]
    fn clopper_pearson_lower_is_zero_without_successes() {
        assert_relative_eq!(clopper_pearson_lower(0, 50, 0.05), 0.0);
    }

    #[test]
    fn clopper_pearson_lower_of_all_successes_is_alpha_to_the_one_over_n() {
        assert_relative_eq!(
            clopper_pearson_lower(50, 50, 0.05),
            0.05_f64.powf(1.0 / 50.0),
            max_relative = 1e-12
        );
    }

    #[test]
    fn pass_probability_is_zero_when_no_count_can_pass() {
        assert_relative_eq!(pass_probability(0.95, 50, 0.05, 0.99), 0.0);
    }

    #[test]
    fn sizing_alternative_prefers_a_declared_rate_then_the_margin_then_midway() {
        let declared = compliance_sizing_alternative(0.95, 0.02, Some(0.98));
        assert_eq!(declared.kind(), AlternativeKind::Declared);
        let margin = compliance_sizing_alternative(0.95, 0.02, None);
        assert_eq!(margin.kind(), AlternativeKind::Margin);
        assert_relative_eq!(margin.rate(), 0.97);
        let midway = compliance_sizing_alternative(0.99, 0.02, None);
        assert_eq!(midway.kind(), AlternativeKind::Midway);
        assert_relative_eq!(midway.rate(), 0.995);
    }

    #[test]
    #[should_panic(expected = "a declared alternative rate must lie in")]
    fn rejects_a_declared_rate_at_the_requirement() {
        let _ = compliance_sizing_alternative(0.95, 0.02, Some(0.95));
    }

    #[test]
    fn sizing_settles_where_the_power_stays_at_target() {
        let sizing = size_compliance(0.90, 0.05, 0.05, 0.80, None, 2_000);
        let required = sizing.required_samples().unwrap();
        let crossing = sizing.first_crossing().unwrap();
        assert!(crossing <= required);
        assert!(sizing.achieved_power().unwrap() >= 0.80);
        for samples in required..=2_000 {
            assert!(pass_probability(0.90, samples, 0.05, 0.95) >= 0.80);
        }
    }

    #[test]
    fn sizing_that_never_settles_reports_no_size() {
        let sizing = size_compliance(0.90, 0.001, 0.05, 0.80, None, 50);
        assert_eq!(sizing.required_samples(), None);
        assert_eq!(sizing.achieved_power(), None);
    }
}
