//! Empirical regression under `regression/fisher` (Statistical Companion §3.4).
//!
//! A test is judged against the baseline it derives from by the one-sided
//! Fisher exact test, expressed as an integer cutoff on the test's success
//! count. For each test count `k_t` the p-value is the hypergeometric lower
//! tail `P(X ≤ k_t)`, `X` the number of the `s = K_b + k_t` pooled successes
//! that fall in the test's `n_t` of the `n_b + n_t` trials; a count fails
//! when its p-value is at most alpha, and the cutoff `c` is the smallest
//! count whose p-value exceeds it. The test passes iff `K_t ≥ c`.
//!
//! The cutoff is monotone in the baseline count — a perfect baseline needs
//! no special case — and the rule never exceeds alpha when the baseline and
//! the test are both random. Everything here is an exact finite sum;
//! nothing is approximated or simulated.
//!
//! Beside the cutoff this module computes what a report discloses about
//! it: the size at the assumed common rate, the design power (baseline and
//! test both yet to be drawn), the resolved power (the baseline observed,
//! its cutoff fixed), the minimum detectable degradation, and the
//! threshold-first inversion (the implied alpha of a declared cutoff, §6.3).

use std::ops::RangeInclusive;

use crate::statistics::distributions::{
    binomial_cdf, binomial_mass, binomial_quantile, bisect, hypergeometric_cdf,
};
use crate::statistics::exact::{at_most_alpha, fisher_p_value_exact};

/// Binomial mass left out of the baseline window on each side: the design
/// power sums over the counts carrying all but a negligible share of the
/// baseline distribution (the truncation moves it by less than `2e-17`).
const WINDOW_MASS: f64 = 1e-17;

/// A declared cutoff whose implied alpha exceeds this is unsound (§6.3).
const SOUND_IMPLIED_ALPHA: f64 = 0.20;

/// Bisection resolution of the minimum detectable degradation.
const DEGRADATION_TOLERANCE: f64 = 1e-12;

/// Bisection resolution of the resolved detectable rate.
const DETECTABLE_RATE_TOLERANCE: f64 = 1e-10;

/// The power at which the minimum detectable degradation is stated (§5.6).
pub const MDD_POWER: f64 = 0.80;

/// The one-sided Fisher p-value `P(X ≤ k_t)` of a test count.
///
/// # Panics
///
/// Panics if a count exceeds its trials.
#[must_use]
pub fn fisher_p_value(
    test_successes: u32,
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
) -> f64 {
    assert!(
        baseline_successes <= baseline_trials,
        "baseline_successes must not exceed baseline_trials"
    );
    assert!(
        test_successes <= test_samples,
        "test_successes must not exceed test_samples"
    );
    let pooled = baseline_successes + test_successes;
    let total = baseline_trials + test_samples;
    hypergeometric_cdf(
        i64::from(test_successes),
        pooled,
        total - pooled,
        test_samples,
    )
}

/// Whether a test count fails: its p-value is at most alpha (inclusive,
/// under the exact-boundary convention).
fn fails(
    test_successes: u32,
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
) -> bool {
    let p_value = fisher_p_value(
        test_successes,
        baseline_successes,
        baseline_trials,
        test_samples,
    );
    at_most_alpha(p_value, alpha, || {
        fisher_p_value_exact(
            test_successes,
            baseline_successes,
            baseline_trials,
            test_samples,
        )
    })
}

/// Validates the inputs every derivation shares.
fn validate(baseline_successes: u32, baseline_trials: u32, test_samples: u32, alpha: f64) {
    assert!(baseline_trials > 0, "baseline_trials must be positive");
    assert!(test_samples > 0, "test_samples must be positive");
    assert!(
        baseline_successes <= baseline_trials,
        "baseline_successes must be in 0..=baseline_trials"
    );
    assert!(
        alpha > 0.0 && alpha < 1.0,
        "alpha must be strictly between 0 and 1, got {alpha}"
    );
}

/// The integer cutoff `c` of `regression/fisher`: PASS iff `K_t ≥ c`.
///
/// The p-value is non-decreasing in the test count, so the cutoff is found
/// by bisection between a virtual failing count below zero and `n_t`.
///
/// # Panics
///
/// Panics if `baseline_trials` or `test_samples` is zero, if
/// `baseline_successes` exceeds `baseline_trials`, or if `alpha` is not in
/// `(0, 1)`.
#[must_use]
// mavai-ref: JVI-9HJ92BC — do not remove (resolves in mavai-orchestrator)
pub fn fisher_cutoff(
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
) -> u32 {
    validate(baseline_successes, baseline_trials, test_samples, alpha);
    // Invariant: `low` fails (or is the virtual -1), `high` passes.
    let (mut low, mut high) = (-1_i64, i64::from(test_samples));
    while high - low > 1 {
        let mid = (low + high) / 2;
        let count = u32::try_from(mid).expect("mid lies in 0..test_samples");
        if fails(
            count,
            baseline_successes,
            baseline_trials,
            test_samples,
            alpha,
        ) {
            low = mid;
        } else {
            high = mid;
        }
    }
    u32::try_from(high).expect("the cutoff lies in 0..=test_samples")
}

/// The cutoff at `test_samples`, walked from a nearby `guess`.
///
/// Sizing searches evaluate the cutoff at one test size after another, or at
/// one baseline count after another; starting from a neighbour's cutoff
/// makes each a step or two of walking rather than a full bisection. The
/// answer is the definition's — the smallest count whose p-value exceeds
/// alpha — whatever the guess.
pub(crate) fn cutoff_near(
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
    guess: u32,
) -> u32 {
    let mut cutoff = guess.min(test_samples);
    while cutoff < test_samples
        && fails(
            cutoff,
            baseline_successes,
            baseline_trials,
            test_samples,
            alpha,
        )
    {
        cutoff += 1;
    }
    while cutoff > 0
        && !fails(
            cutoff - 1,
            baseline_successes,
            baseline_trials,
            test_samples,
            alpha,
        )
    {
        cutoff -= 1;
    }
    cutoff
}

/// The cutoffs for every baseline count in `counts` (ascending), each
/// walked from its predecessor's — the cutoff is non-decreasing in the
/// baseline count.
pub(crate) fn fisher_cutoffs(
    counts: &RangeInclusive<u32>,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
) -> Vec<u32> {
    let mut cutoffs = Vec::with_capacity(counts.clone().count());
    let mut previous: Option<u32> = None;
    for count in counts.clone() {
        let cutoff = previous.map_or_else(
            || fisher_cutoff(count, baseline_trials, test_samples, alpha),
            |guess| cutoff_near(count, baseline_trials, test_samples, alpha, guess),
        );
        cutoffs.push(cutoff);
        previous = Some(cutoff);
    }
    cutoffs
}

/// The cutoffs for every baseline count in `counts`, each walked from the
/// corresponding entry of `guesses`.
pub(crate) fn fisher_cutoffs_near(
    counts: &RangeInclusive<u32>,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
    guesses: &[u32],
) -> Vec<u32> {
    counts
        .clone()
        .zip(guesses)
        .map(|(count, &guess)| cutoff_near(count, baseline_trials, test_samples, alpha, guess))
        .collect()
}

/// The baseline counts carrying all but `2e-17` of `Bin(n_b, rate)`.
///
/// # Panics
///
/// Panics if `rate` is outside `[0, 1]`.
#[must_use]
pub(crate) fn baseline_window(baseline_trials: u32, rate: f64) -> RangeInclusive<u32> {
    assert!(
        (0.0..=1.0).contains(&rate),
        "rate must be in [0, 1], got {rate}"
    );
    if rate >= 1.0 {
        return baseline_trials..=baseline_trials;
    }
    if rate <= 0.0 {
        return 0..=0;
    }
    let low = binomial_quantile(WINDOW_MASS, baseline_trials, rate);
    let high = baseline_trials - binomial_quantile(WINDOW_MASS, baseline_trials, 1.0 - rate);
    low..=high
}

/// `Σ_k P_{p_b}(K_b = k) P_{p_t}(K_t < c(k))` over the window's counts.
pub(crate) fn fail_probability(
    cutoffs: &[u32],
    counts: &RangeInclusive<u32>,
    baseline_trials: u32,
    baseline_rate: f64,
    test_samples: u32,
    test_rate: f64,
) -> f64 {
    counts
        .clone()
        .zip(cutoffs)
        .map(|(count, &cutoff)| {
            binomial_mass(count, baseline_trials, baseline_rate)
                * binomial_cdf(i64::from(cutoff) - 1, test_samples, test_rate)
        })
        .sum()
}

/// The cutoff of `regression/fisher` for one configuration, and what a
/// report discloses about it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegressionDerivation {
    cutoff: u32,
    test_samples: u32,
    alpha: f64,
    size_at_assumed_common_rate: Option<f64>,
}

impl RegressionDerivation {
    /// The binding decision artefact: PASS iff `K_t ≥ cutoff`.
    #[must_use]
    pub const fn cutoff(&self) -> u32 {
        self.cutoff
    }

    /// `n_t`, the size the cutoff was derived at.
    #[must_use]
    pub const fn test_samples(&self) -> u32 {
        self.test_samples
    }

    /// The one-sided level.
    #[must_use]
    pub const fn alpha(&self) -> f64 {
        self.alpha
    }

    /// The exact unconditional false-degradation-signal probability were the
    /// unknown common rate equal to the baseline's observed rate — a
    /// property of the procedure at that rate, not of the run; `None` when
    /// the baseline rate is 0 or 1, where it is degenerate.
    #[must_use]
    pub const fn size_at_assumed_common_rate(&self) -> Option<f64> {
        self.size_at_assumed_common_rate
    }

    /// The cutoff as a rate, `c / n_t` — the displayed threshold.
    #[must_use]
    pub fn threshold_real(&self) -> f64 {
        f64::from(self.cutoff) / f64::from(self.test_samples)
    }

    /// `c / n_t` rounded to six places, as a report displays it.
    #[must_use]
    pub fn displayed_rate(&self) -> f64 {
        (self.threshold_real() * 1e6).round() / 1e6
    }
}

/// Derives the cutoff of `regression/fisher` and its informational size.
///
/// The design rule that a test may not exceed its baseline is judged by the
/// caller before any derivation
/// ([`check_test_size`](crate::statistics::rules::check_test_size)).
///
/// # Panics
///
/// Panics on the inputs [`fisher_cutoff`] rejects.
#[must_use]
pub fn derive_regression_cutoff(
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
) -> RegressionDerivation {
    RegressionDerivation {
        cutoff: fisher_cutoff(baseline_successes, baseline_trials, test_samples, alpha),
        test_samples,
        alpha,
        size_at_assumed_common_rate: size_at_assumed_common_rate(
            baseline_successes,
            baseline_trials,
            test_samples,
            alpha,
        ),
    }
}

/// The procedure's false-degradation-signal probability at the common rate
/// `p = K_b / n_b`; `None` at a baseline rate of 0 or 1, where it is
/// degenerate.
///
/// # Panics
///
/// Panics on the inputs [`fisher_cutoff`] rejects.
#[must_use]
pub fn size_at_assumed_common_rate(
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
) -> Option<f64> {
    validate(baseline_successes, baseline_trials, test_samples, alpha);
    if baseline_successes == 0 || baseline_successes == baseline_trials {
        return None;
    }
    let rate = f64::from(baseline_successes) / f64::from(baseline_trials);
    Some(design_power(
        baseline_trials,
        test_samples,
        alpha,
        rate,
        rate,
    ))
}

/// The exact power of `regression/fisher` with the baseline yet to be drawn.
///
/// `Σ_k P_{p0}(K_b = k) P_{p_design}(K_t < c(k))`: the probability that a
/// service truly at the design alternative rate fails the test, averaged
/// over the baseline counts a baseline of `n_b` at `p0` could return. At
/// `p_design = p0` it is the size at that common rate.
///
/// # Panics
///
/// Panics if a size is zero, a rate is outside `[0, 1]`, or `alpha` is not
/// in `(0, 1)`.
#[must_use]
// mavai-ref: JVI-EGMJ0MU — do not remove (resolves in mavai-orchestrator)
pub fn design_power(
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
    baseline_rate: f64,
    design_alternative_rate: f64,
) -> f64 {
    let counts = baseline_window(baseline_trials, baseline_rate);
    let cutoffs = fisher_cutoffs(&counts, baseline_trials, test_samples, alpha);
    fail_probability(
        &cutoffs,
        &counts,
        baseline_trials,
        baseline_rate,
        test_samples,
        design_alternative_rate,
    )
}

/// The power of the test resolved against an observed baseline.
///
/// The observed `K_b` fixes the cutoff, so the power at the design
/// alternative rate is `P_{p_design}(K_t < c(K_b))`. It answers a different
/// question from [`design_power`] and is reported beside it, named apart.
///
/// # Panics
///
/// Panics on the inputs [`fisher_cutoff`] rejects, or a rate outside
/// `[0, 1]`.
#[must_use]
pub fn resolved_power(
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
    design_alternative_rate: f64,
) -> f64 {
    let cutoff = fisher_cutoff(baseline_successes, baseline_trials, test_samples, alpha);
    binomial_cdf(i64::from(cutoff) - 1, test_samples, design_alternative_rate)
}

/// The smallest drop the design detects with `power`: the inversion of the
/// design power.
///
/// The smallest `δ` at which the design power against `p_b − δ` reaches
/// `power`, by bisection to `1e-12`; `None` when even a test rate of 0 falls
/// short — no degradation is detectable at that power.
///
/// # Panics
///
/// Panics if a size is zero, `baseline_rate` is outside `[0, 1]`, or
/// `alpha` is not in `(0, 1)`.
#[must_use]
pub fn minimum_detectable_degradation(
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
    baseline_rate: f64,
    power: f64,
) -> Option<f64> {
    let counts = baseline_window(baseline_trials, baseline_rate);
    let cutoffs = fisher_cutoffs(&counts, baseline_trials, test_samples, alpha);
    let power_at = |drop: f64| {
        fail_probability(
            &cutoffs,
            &counts,
            baseline_trials,
            baseline_rate,
            test_samples,
            (baseline_rate - drop).max(0.0),
        )
    };
    if power_at(baseline_rate) < power {
        return None;
    }
    let (_, high) = bisect(0.0, baseline_rate, DEGRADATION_TOLERANCE, |drop| {
        power_at(drop) >= power
    });
    Some(high)
}

/// The largest true rate the resolved test detects with `power`.
///
/// Inverts the resolved power, the cutoff fixed by the observed baseline:
/// the largest `p` with `P_p(K_t < c) ≥ power`, by bisection to `1e-10`;
/// `None` when not even a rate of 0 reaches it (a cutoff of 0 fails
/// nothing).
///
/// # Panics
///
/// Panics on the inputs [`fisher_cutoff`] rejects.
#[must_use]
pub fn resolved_detectable_rate(
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
    alpha: f64,
    power: f64,
) -> Option<f64> {
    let cutoff = fisher_cutoff(baseline_successes, baseline_trials, test_samples, alpha);
    let power_at = |rate: f64| binomial_cdf(i64::from(cutoff) - 1, test_samples, rate);
    if power_at(0.0) < power {
        return None;
    }
    let (low, _) = bisect(0.0, 1.0, DETECTABLE_RATE_TOLERANCE, |rate| {
        power_at(rate) < power
    });
    Some(low)
}

/// The threshold-first inversion of a declared cutoff (§6.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImpliedAlpha {
    alpha: Option<f64>,
    is_sound: Option<bool>,
}

impl ImpliedAlpha {
    /// The smallest alpha at which `regression/fisher` yields the cutoff (0
    /// for a cutoff of 0, the infimum); `None` when no alpha yields it — the
    /// rule skips that cutoff.
    #[must_use]
    pub const fn alpha(&self) -> Option<f64> {
        self.alpha
    }

    /// Whether the implied alpha is at most 0.20; `None` when there is no
    /// implied alpha.
    #[must_use]
    pub const fn is_sound(&self) -> Option<bool> {
        self.is_sound
    }
}

/// The implied alpha of a declared cutoff under `regression/fisher`.
///
/// The rule gives `c` exactly when `P(X ≤ c − 1) ≤ alpha < P(X ≤ c)`, so
/// the implied alpha is the p-value at `c − 1`.
///
/// # Panics
///
/// Panics if `cutoff` exceeds `test_samples`, or on counts out of range.
#[must_use]
// mavai-ref: JVI-HHV7KT0 — do not remove (resolves in mavai-orchestrator)
pub fn implied_alpha(
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
    cutoff: u32,
) -> ImpliedAlpha {
    assert!(
        cutoff <= test_samples,
        "cutoff must be between 0 and test_samples"
    );
    if cutoff == 0 {
        return ImpliedAlpha {
            alpha: Some(0.0),
            is_sound: Some(true),
        };
    }
    let below = fisher_p_value(
        cutoff - 1,
        baseline_successes,
        baseline_trials,
        test_samples,
    );
    let at = fisher_p_value(cutoff, baseline_successes, baseline_trials, test_samples);
    if below >= at {
        return ImpliedAlpha {
            alpha: None,
            is_sound: None,
        };
    }
    ImpliedAlpha {
        alpha: Some(below),
        is_sound: Some(below <= SOUND_IMPLIED_ALPHA),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn worked_example_cutoff_is_91_of_100() {
        // Companion §3.4: baseline 951 of 1000, test of 100 at alpha 0.05.
        assert_eq!(fisher_cutoff(951, 1000, 100, 0.05), 91);
    }

    #[test]
    fn a_count_below_the_cutoff_fails_and_the_cutoff_passes() {
        let cutoff = fisher_cutoff(951, 1000, 100, 0.05);
        assert!(fisher_p_value(cutoff - 1, 951, 1000, 100) <= 0.05);
        assert!(fisher_p_value(cutoff, 951, 1000, 100) > 0.05);
    }

    #[test]
    fn a_perfect_baseline_needs_no_special_case() {
        let cutoff = fisher_cutoff(100, 100, 100, 0.05);
        assert!(cutoff < 100);
        assert!(cutoff > 90);
    }

    #[test]
    fn a_zero_baseline_passes_on_nothing_observed() {
        assert_eq!(fisher_cutoff(0, 100, 50, 0.05), 0);
    }

    #[test]
    fn the_cutoff_rises_with_the_baseline_count() {
        let low = fisher_cutoff(900, 1000, 100, 0.05);
        let high = fisher_cutoff(990, 1000, 100, 0.05);
        assert!(high >= low);
    }

    #[test]
    fn walked_cutoffs_agree_with_bisection() {
        let counts = 940..=960;
        let walked = fisher_cutoffs(&counts, 1000, 100, 0.05);
        for (count, walked) in counts.zip(walked) {
            assert_eq!(walked, fisher_cutoff(count, 1000, 100, 0.05));
        }
        assert_eq!(cutoff_near(951, 1000, 100, 0.05, 0), 91);
        assert_eq!(cutoff_near(951, 1000, 100, 0.05, 100), 91);
    }

    #[test]
    fn derivation_reports_the_threshold_as_a_rate() {
        let derivation = derive_regression_cutoff(951, 1000, 100, 0.05);
        assert_eq!(derivation.cutoff(), 91);
        assert_relative_eq!(derivation.threshold_real(), 0.91);
        assert_relative_eq!(derivation.displayed_rate(), 0.91);
        let size = derivation.size_at_assumed_common_rate().unwrap();
        assert!(size > 0.0 && size <= 0.05);
    }

    #[test]
    fn size_is_undefined_at_a_degenerate_baseline_rate() {
        assert!(size_at_assumed_common_rate(100, 100, 50, 0.05).is_none());
        assert!(size_at_assumed_common_rate(0, 100, 50, 0.05).is_none());
    }

    #[test]
    fn design_power_at_the_common_rate_is_the_size() {
        let size = size_at_assumed_common_rate(951, 1000, 100, 0.05).unwrap();
        assert_relative_eq!(
            design_power(1000, 100, 0.05, 0.951, 0.951),
            size,
            max_relative = 1e-12
        );
    }

    #[test]
    fn design_power_grows_as_the_alternative_falls() {
        let near = design_power(1000, 100, 0.05, 0.95, 0.93);
        let far = design_power(1000, 100, 0.05, 0.95, 0.85);
        assert!(far > near);
    }

    #[test]
    fn resolved_power_is_the_binomial_tail_below_the_cutoff() {
        let expected = binomial_cdf(90, 100, 0.90);
        assert_relative_eq!(resolved_power(951, 1000, 100, 0.05, 0.90), expected);
    }

    #[test]
    fn minimum_detectable_degradation_reaches_the_power() {
        let drop = minimum_detectable_degradation(1000, 100, 0.05, 0.95, MDD_POWER).unwrap();
        let power = design_power(1000, 100, 0.05, 0.95, 0.95 - drop);
        assert!(power >= MDD_POWER - 1e-9);
    }

    #[test]
    fn no_degradation_is_detectable_by_a_test_that_cannot_fail() {
        assert!(minimum_detectable_degradation(10, 1, 0.001, 0.5, MDD_POWER).is_none());
    }

    #[test]
    fn resolved_detectable_rate_inverts_the_resolved_power() {
        let rate = resolved_detectable_rate(951, 1000, 100, 0.05, MDD_POWER).unwrap();
        assert_relative_eq!(
            resolved_power(951, 1000, 100, 0.05, rate),
            MDD_POWER,
            epsilon = 1e-8
        );
    }

    #[test]
    fn implied_alpha_of_the_derived_cutoff_is_at_most_the_level() {
        let implied = implied_alpha(951, 1000, 100, 91);
        let alpha = implied.alpha().unwrap();
        assert!(alpha <= 0.05);
        assert_eq!(implied.is_sound(), Some(true));
        assert_eq!(fisher_cutoff(951, 1000, 100, alpha), 91);
    }

    #[test]
    fn implied_alpha_of_a_zero_cutoff_is_zero() {
        let implied = implied_alpha(951, 1000, 100, 0);
        assert_eq!(implied.alpha(), Some(0.0));
        assert_eq!(implied.is_sound(), Some(true));
    }

    #[test]
    fn a_cutoff_close_to_the_baseline_rate_is_unsound() {
        assert_eq!(implied_alpha(951, 1000, 100, 95).is_sound(), Some(false));
    }

    #[test]
    #[should_panic(expected = "test_samples must be positive")]
    fn rejects_a_zero_test_size() {
        let _ = fisher_cutoff(90, 100, 0, 0.05);
    }

    #[test]
    #[should_panic(expected = "alpha must be strictly between 0 and 1")]
    fn rejects_an_alpha_outside_the_unit_interval() {
        let _ = fisher_cutoff(90, 100, 50, 1.0);
    }
}
