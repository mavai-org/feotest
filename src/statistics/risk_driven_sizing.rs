//! Risk-driven sizing of a regression test against the operative rule
//! (Statistical Companion §5.4.1).
//!
//! A regression test's cutoff is derived at the test's own size, so sizing
//! is done against the cutoff the test will actually apply. Two operations
//! answer different questions and are named apart:
//!
//! - **Design sizing**, before the baseline exists: the baseline is planned
//!   at `n_b` trials and an expected rate `p0`, and the design power
//!   averages over the baseline count yet to be drawn.
//! - **Resolved sizing**, against an existing baseline: its observed count
//!   fixes the cutoff for every candidate test size, and the resolved power
//!   decides. Once a baseline exists, `BASELINE_TOO_SMALL` is judged by it.
//!
//! The **design alternative rate** `p_design` is the true rate at which the
//! test must reach its target power — a declared design input, not a
//! measured estimate and not a tolerance: the test still flags any
//! degradation from the baseline, including one to a rate above
//! `p_design`.
//!
//! Exact power is a sawtooth in the test size, so the required size is the
//! smallest `n_t` from which power *stays* at or above the target for every
//! larger test up to the baseline size — not the first crossing — subject
//! to the design rule `n_t ≤ n_b`.

use crate::statistics::distributions::{binomial_cdf, bisect};
use crate::statistics::regression::{
    baseline_window, cutoff_near, design_power, fail_probability, fisher_cutoff, fisher_cutoffs,
    fisher_cutoffs_near,
};

/// Bisection resolution for the detectable-rate inversion.
const DETECTABLE_RATE_TOLERANCE: f64 = 1e-10;

/// Why a sizing design cannot be priced.
///
/// Carried as a value so a caller — a pre-flight check, a report, a
/// conformance run — can tell the refusals apart without parsing a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizingRefusal {
    /// The baseline observed (or expects) no successes (§4.3.4): there is no
    /// rate below it to detect. *Measure a baseline before sizing against
    /// it.*
    ZeroBaseline,
    /// The design alternative rate is not below the baseline rate, so there
    /// is no degradation to detect. *Re-measure rather than raise the
    /// rate.*
    AlternativeNotBelowBaseline,
    /// A candidate test larger than the baseline the design rule admits.
    TestLargerThanBaseline,
    /// No test the design rule admits reaches and holds the target power: a
    /// larger baseline is needed.
    BaselineTooSmall,
}

impl SizingRefusal {
    /// The stable identifier the oracle's fixtures name this refusal by.
    #[must_use]
    pub const fn category(self) -> &'static str {
        match self {
            Self::ZeroBaseline => "ZERO_BASELINE",
            Self::AlternativeNotBelowBaseline => "ALTERNATIVE_NOT_BELOW_BASELINE",
            Self::TestLargerThanBaseline => "TEST_LARGER_THAN_BASELINE",
            Self::BaselineTooSmall => "BASELINE_TOO_SMALL",
        }
    }

    /// The operator-facing explanation, naming the corrective action.
    #[must_use]
    pub fn message(self, baseline_rate: f64, design_alternative_rate: f64) -> String {
        match self {
            Self::ZeroBaseline => "the baseline rate is exactly 0: the baseline observed no \
                 successes, so there is no rate below it to detect and no sample size can \
                 price this design. Measure a baseline with at least one success before \
                 sizing against it."
                .to_owned(),
            Self::AlternativeNotBelowBaseline => format!(
                "the design alternative rate ({design_alternative_rate}) must sit below the \
                 baseline rate ({baseline_rate}): it is the true rate at which the test must \
                 detect a degradation; to demand more than the baseline delivered, re-measure \
                 the baseline rather than raising the rate"
            ),
            Self::TestLargerThanBaseline => "the test is larger than the baseline it consumes; \
                 the baseline must be at least as large as any test that consumes it"
                .to_owned(),
            Self::BaselineTooSmall => format!(
                "no test up to the size of the baseline reaches and holds the target power \
                 against a design alternative rate of {design_alternative_rate} (baseline \
                 rate {baseline_rate}): measure a larger baseline, declare a lower design \
                 alternative rate, or accept a lower power"
            ),
        }
    }
}

/// The refusal a sizing design meets before any power is computed, or
/// `None`.
///
/// # Panics
///
/// Panics if `baseline_rate` is outside `[0, 1]`.
#[must_use]
pub fn check_sizing_domain(
    baseline_rate: f64,
    baseline_trials: u32,
    design_alternative_rate: Option<f64>,
    test_samples: Option<u32>,
) -> Option<SizingRefusal> {
    assert!(
        (0.0..=1.0).contains(&baseline_rate),
        "baseline_rate must be in [0, 1], got {baseline_rate}"
    );
    if baseline_rate == 0.0 {
        return Some(SizingRefusal::ZeroBaseline);
    }
    if design_alternative_rate.is_some_and(|rate| rate >= baseline_rate) {
        return Some(SizingRefusal::AlternativeNotBelowBaseline);
    }
    if test_samples.is_some_and(|n| n > baseline_trials) {
        return Some(SizingRefusal::TestLargerThanBaseline);
    }
    None
}

/// A required test size and the power there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignSizing {
    required_samples: u32,
    power: f64,
}

impl DesignSizing {
    /// The smallest test size from which the design power stays at target.
    #[must_use]
    pub const fn required_samples(&self) -> u32 {
        self.required_samples
    }

    /// The design power at the required size.
    #[must_use]
    pub const fn power(&self) -> f64 {
        self.power
    }
}

/// Design sizing: the smallest `n_t ≤ n_b` from which design power stays at
/// target.
///
/// Scans down from `n_b` to the first size whose power falls short; the
/// answer is the next size up. `None` (`BASELINE_TOO_SMALL`) when the power
/// at `n_b` itself is short. The domain is the caller's to check first
/// ([`check_sizing_domain`]).
///
/// # Panics
///
/// Panics if `baseline_trials` is zero, a rate is outside `[0, 1]`, or
/// `alpha` is not in `(0, 1)`.
#[must_use]
pub fn design_required_samples(
    baseline_rate: f64,
    baseline_trials: u32,
    design_alternative_rate: f64,
    alpha: f64,
    target_power: f64,
) -> Option<DesignSizing> {
    let counts = baseline_window(baseline_trials, baseline_rate);
    let mut cutoffs = fisher_cutoffs(&counts, baseline_trials, baseline_trials, alpha);
    let mut held: Option<DesignSizing> = None;
    for test_samples in (1..=baseline_trials).rev() {
        if test_samples < baseline_trials {
            let guesses: Vec<u32> = cutoffs
                .iter()
                .map(|&c| scaled_guess(c, test_samples, test_samples + 1))
                .collect();
            cutoffs = fisher_cutoffs_near(&counts, baseline_trials, test_samples, alpha, &guesses);
        }
        let power = fail_probability(
            &cutoffs,
            &counts,
            baseline_trials,
            baseline_rate,
            test_samples,
            design_alternative_rate,
        );
        if power < target_power {
            return held;
        }
        held = Some(DesignSizing {
            required_samples: test_samples,
            power,
        });
    }
    held
}

/// A cutoff at one size, scaled to a neighbouring size as a walking guess.
fn scaled_guess(cutoff: u32, to: u32, from: u32) -> u32 {
    let scaled = u64::from(cutoff) * u64::from(to) + u64::from(from) / 2;
    u32::try_from(scaled / u64::from(from)).expect("a scaled cutoff never exceeds its size")
}

/// The largest design alternative rate detectable at the target design
/// power.
///
/// Power falls as `p_design` rises toward `p0`, so bisection over `(0, p0)`
/// to `1e-10`; `None` when even `p_design = 0` falls short.
///
/// # Panics
///
/// Panics if a size is zero, `baseline_rate` is outside `[0, 1]`, or
/// `alpha` is not in `(0, 1)`.
#[must_use]
pub fn design_detectable_rate(
    test_samples: u32,
    baseline_rate: f64,
    baseline_trials: u32,
    alpha: f64,
    target_power: f64,
) -> Option<f64> {
    let counts = baseline_window(baseline_trials, baseline_rate);
    let cutoffs = fisher_cutoffs(&counts, baseline_trials, test_samples, alpha);
    let power_at = |rate: f64| {
        fail_probability(
            &cutoffs,
            &counts,
            baseline_trials,
            baseline_rate,
            test_samples,
            rate,
        )
    };
    if power_at(0.0) < target_power {
        return None;
    }
    let (low, _) = bisect(0.0, baseline_rate, DETECTABLE_RATE_TOLERANCE, |rate| {
        power_at(rate) < target_power
    });
    Some(low)
}

/// The design power at a candidate test size
/// (see [`design_power`]).
///
/// # Panics
///
/// Panics on the inputs `design_power` rejects.
#[must_use]
pub fn design_power_at(
    test_samples: u32,
    baseline_rate: f64,
    baseline_trials: u32,
    design_alternative_rate: f64,
    alpha: f64,
) -> f64 {
    design_power(
        baseline_trials,
        test_samples,
        alpha,
        baseline_rate,
        design_alternative_rate,
    )
}

/// The required test size under resolved sizing against an observed
/// baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedSizing {
    required_samples: u32,
    power: f64,
    first_crossing: u32,
}

impl ResolvedSizing {
    /// The smallest `n_t` from which the resolved power stays at or above
    /// the target up to `n_b`.
    #[must_use]
    pub const fn required_samples(&self) -> u32 {
        self.required_samples
    }

    /// The resolved power at [`required_samples`](Self::required_samples).
    #[must_use]
    pub const fn power(&self) -> f64 {
        self.power
    }

    /// The smallest `n_t` whose resolved power first reaches the target —
    /// reported, never the answer.
    #[must_use]
    pub const fn first_crossing(&self) -> u32 {
        self.first_crossing
    }
}

/// The cutoff against an observed baseline at every test size `1..=n_b`.
///
/// Element `i` is the cutoff at `n_t = i + 1`; each size's cutoff is walked
/// from its predecessor's, one or two p-values apart.
///
/// # Panics
///
/// Panics on the inputs
/// [`fisher_cutoff`] rejects.
#[must_use]
pub fn resolved_cutoffs(baseline_successes: u32, baseline_trials: u32, alpha: f64) -> Vec<u32> {
    let mut cutoffs = Vec::with_capacity(baseline_trials as usize);
    let mut cutoff = fisher_cutoff(baseline_successes, baseline_trials, 1, alpha);
    cutoffs.push(cutoff);
    for test_samples in 2..=baseline_trials {
        cutoff = cutoff_near(
            baseline_successes,
            baseline_trials,
            test_samples,
            alpha,
            cutoff,
        );
        cutoffs.push(cutoff);
    }
    cutoffs
}

/// Resolved sizing: `None` (`BASELINE_TOO_SMALL`) when no `n_t ≤ n_b`
/// reaches and holds the target. The domain is the caller's to check first
/// ([`check_sizing_domain`]).
///
/// # Panics
///
/// Panics on the inputs
/// [`fisher_cutoff`] rejects,
/// or a rate outside `[0, 1]`.
#[must_use]
pub fn resolved_sizing(
    baseline_successes: u32,
    baseline_trials: u32,
    design_alternative_rate: f64,
    alpha: f64,
    target_power: f64,
) -> Option<ResolvedSizing> {
    let cutoffs = resolved_cutoffs(baseline_successes, baseline_trials, alpha);
    let powers: Vec<f64> = (1..=baseline_trials)
        .zip(&cutoffs)
        .map(|(test_samples, &cutoff)| {
            binomial_cdf(i64::from(cutoff) - 1, test_samples, design_alternative_rate)
        })
        .collect();
    let last_short = powers.iter().rposition(|&p| p < target_power);
    let start = match last_short {
        Some(index) if index == powers.len() - 1 => return None,
        Some(index) => index + 1,
        None => 0,
    };
    let crossing = powers
        .iter()
        .position(|&p| p >= target_power)
        .expect("the power at the required size reaches the target");
    Some(ResolvedSizing {
        required_samples: size_at(start),
        power: powers[start],
        first_crossing: size_at(crossing),
    })
}

/// The test size at a zero-based position of the size scan.
fn size_at(index: usize) -> u32 {
    u32::try_from(index + 1).expect("the scan never exceeds the baseline size")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statistics::regression::resolved_power;
    use approx::assert_relative_eq;

    #[test]
    fn domain_refusals_are_named_in_order() {
        assert_eq!(
            check_sizing_domain(0.0, 100, Some(0.5), None),
            Some(SizingRefusal::ZeroBaseline)
        );
        assert_eq!(
            check_sizing_domain(0.9, 100, Some(0.9), None),
            Some(SizingRefusal::AlternativeNotBelowBaseline)
        );
        assert_eq!(
            check_sizing_domain(0.9, 100, Some(0.8), Some(200)),
            Some(SizingRefusal::TestLargerThanBaseline)
        );
        assert_eq!(check_sizing_domain(0.9, 100, Some(0.8), Some(100)), None);
    }

    #[test]
    fn refusal_categories_are_stable() {
        assert_eq!(
            SizingRefusal::BaselineTooSmall.category(),
            "BASELINE_TOO_SMALL"
        );
        assert!(
            SizingRefusal::AlternativeNotBelowBaseline
                .message(0.9, 0.95)
                .contains("re-measure")
        );
    }

    #[test]
    fn scaled_guess_rounds_to_the_nearest_count() {
        assert_eq!(scaled_guess(91, 99, 100), 90);
        assert_eq!(scaled_guess(0, 5, 6), 0);
    }

    #[test]
    fn design_sizing_holds_the_power_from_the_required_size_up() {
        let sizing = design_required_samples(0.9, 1000, 0.8, 0.05, 0.8).unwrap();
        assert!(sizing.power() >= 0.8);
        let below = design_power_at(sizing.required_samples() - 1, 0.9, 1000, 0.8, 0.05);
        assert!(below < 0.8);
    }

    #[test]
    fn a_design_that_cannot_reach_the_power_is_too_small() {
        assert!(design_required_samples(0.96, 300, 0.93, 0.05, 0.8).is_none());
    }

    #[test]
    fn detectable_rate_meets_the_target_power() {
        let rate = design_detectable_rate(100, 0.87, 3000, 0.05, 0.8).unwrap();
        assert!(rate < 0.87);
        assert!(design_power_at(100, 0.87, 3000, rate, 0.05) >= 0.8 - 1e-6);
    }

    #[test]
    fn resolved_cutoffs_agree_with_direct_derivation() {
        let cutoffs = resolved_cutoffs(95, 100, 0.05);
        for (index, cutoff) in cutoffs.iter().enumerate() {
            let size = u32::try_from(index + 1).unwrap();
            assert_eq!(*cutoff, fisher_cutoff(95, 100, size, 0.05));
        }
    }

    #[test]
    fn resolved_sizing_reports_the_power_at_the_required_size() {
        let sizing = resolved_sizing(1920, 2000, 0.93, 0.05, 0.8).unwrap();
        assert!(sizing.first_crossing() <= sizing.required_samples());
        assert_relative_eq!(
            sizing.power(),
            resolved_power(1920, 2000, sizing.required_samples(), 0.05, 0.93),
            max_relative = 1e-12
        );
    }

    #[test]
    fn a_baseline_too_small_for_resolved_sizing_is_refused() {
        assert!(resolved_sizing(288, 300, 0.93, 0.05, 0.8).is_none());
    }
}
