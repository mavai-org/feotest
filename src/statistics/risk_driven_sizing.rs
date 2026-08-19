//! Risk-driven sample sizing against a moving acceptance floor.
//!
//! A baseline-derived test does not judge against a fixed threshold: its
//! acceptance floor is the Wilson lower bound of the baseline rate computed
//! *at the test's own sample size*, so the floor falls as the sample shrinks
//! — a small sample proves less, so less is demanded of it. The closed-form
//! power calculations in [`sample_size`](crate::statistics::sample_size)
//! hold the threshold constant and therefore overstate the power of small
//! designs. The functions here put the moving floor inside the calculation.
//!
//! The caller declares a **minimum acceptable rate** — the worst true
//! success rate they are willing to tolerate; a declared bound, not a
//! measured estimate. [`self_consistent_power`] prices the probability that
//! a service truly at that rate fails a test of a given size;
//! [`required_sample_size`] finds the smallest size meeting a target power;
//! [`detectable_rate`] inverts the question for a fixed, affordable size.
//!
//! The floor and the power calculation share one z convention: the floor is
//! the same one-sided Wilson lower bound used for threshold derivation
//! throughout the statistics module.

use statrs::distribution::{ContinuousCDF, Normal};

use crate::statistics::proportion;
use crate::statistics::types::ConfidenceLevel;

/// The search cap for [`required_sample_size`]: a requirement beyond this is
/// treated as a misconfigured tolerance, not a plan.
const REQUIRED_SAMPLE_SIZE_CAP: u32 = 10_000_000;

/// Returns the standard normal distribution N(0, 1).
///
/// # Panics
///
/// Cannot panic — parameters are compile-time constants.
fn standard_normal() -> Normal {
    Normal::new(0.0, 1.0).unwrap()
}

/// Computes the self-consistent power of a test of `sample_size` samples.
///
/// The acceptance floor is the one-sided Wilson lower bound of
/// `baseline_rate` at `sample_size` itself — the bar this test would
/// actually apply. The result is the probability that a service whose true
/// rate is `minimum_acceptable_rate` fails the test, i.e. that a
/// degradation at least that severe is detected:
///
/// Power(n) = Φ((floor(n) − `p_min`) / √(`p_min` × (1 − `p_min`) / n))
///
/// # Panics
///
/// Panics if `sample_size` is zero, if `baseline_rate` is not in (0, 1), or
/// if `minimum_acceptable_rate` is not in (0, `baseline_rate`). The
/// construction is defined only for a tolerance strictly below the measured
/// baseline rate: a tolerance at or above it asks the test to detect a
/// "degradation" the baseline already exceeds — re-measure the baseline
/// rather than asserting improvement through the tolerance.
#[must_use]
pub fn self_consistent_power(
    sample_size: u32,
    baseline_rate: f64,
    minimum_acceptable_rate: f64,
    confidence: ConfidenceLevel,
) -> f64 {
    assert!(sample_size > 0, "sample_size must be positive");
    assert_sizing_domain(baseline_rate, minimum_acceptable_rate);

    let floor = proportion::lower_bound_from_rate(baseline_rate, sample_size, confidence);
    let n = f64::from(sample_size);
    let standard_error = (minimum_acceptable_rate * (1.0 - minimum_acceptable_rate) / n).sqrt();
    standard_normal().cdf((floor - minimum_acceptable_rate) / standard_error)
}

/// Computes the smallest sample size whose self-consistent power meets
/// `target_power`.
///
/// Within the domain, growing the sample both raises the acceptance floor
/// toward the baseline rate and shrinks the standard error, so the power is
/// increasing in the sample size and the minimum is well defined. It is
/// found by doubling until the target is met, then bisecting.
///
/// # Panics
///
/// Panics if `baseline_rate` is not in (0, 1), if `minimum_acceptable_rate`
/// is not in (0, `baseline_rate`) (see [`self_consistent_power`] for the
/// domain rationale), if `target_power` is not in (0, 1), or if the
/// requirement exceeds 10,000,000 samples — a tolerance that tight against
/// that baseline is a misconfiguration, not a plan.
#[must_use]
pub fn required_sample_size(
    baseline_rate: f64,
    minimum_acceptable_rate: f64,
    confidence: ConfidenceLevel,
    target_power: f64,
) -> u32 {
    assert_sizing_domain(baseline_rate, minimum_acceptable_rate);
    assert!(
        target_power > 0.0 && target_power < 1.0,
        "target_power must be in (0, 1), got {target_power}"
    );

    let power_of =
        |n: u32| self_consistent_power(n, baseline_rate, minimum_acceptable_rate, confidence);

    let mut upper = 1;
    #[allow(
        clippy::while_float,
        reason = "power increases toward 1 with n; the cap bounds the loop"
    )]
    while power_of(upper) < target_power {
        upper *= 2;
        assert!(
            upper <= REQUIRED_SAMPLE_SIZE_CAP,
            "required sample size exceeds {REQUIRED_SAMPLE_SIZE_CAP}: \
             minimum_acceptable_rate ({minimum_acceptable_rate}) is too close to \
             baseline_rate ({baseline_rate}) to detect at power {target_power}"
        );
    }
    if upper == 1 {
        return 1;
    }

    // Invariant: power(lower) < target_power <= power(upper).
    let mut lower = upper / 2;
    while lower + 1 < upper {
        let mid = lower + (upper - lower) / 2;
        (lower, upper) = if power_of(mid) >= target_power {
            (lower, mid)
        } else {
            (mid, upper)
        };
    }
    upper
}

/// Computes the largest tolerable true rate detectable at `target_power`
/// with `sample_size` samples.
///
/// This is the inversion of [`required_sample_size`] for a fixed,
/// affordable sample size: the highest minimum acceptable rate (the
/// smallest drop from the baseline) at which the self-consistent power
/// still meets the target. Found by bisection over (0, `baseline_rate`) to
/// an absolute tolerance of 1e-10.
///
/// # Panics
///
/// Panics if `sample_size` is zero, if `baseline_rate` is not in (0, 1), or
/// if `target_power` is not in (0, 1).
#[must_use]
pub fn detectable_rate(
    sample_size: u32,
    baseline_rate: f64,
    confidence: ConfidenceLevel,
    target_power: f64,
) -> f64 {
    assert!(sample_size > 0, "sample_size must be positive");
    assert!(
        baseline_rate != 0.0,
        "{}",
        SizingRefusal::ZeroBaseline.message(baseline_rate, f64::NAN)
    );
    assert!(
        baseline_rate > 0.0 && baseline_rate < 1.0,
        "baseline_rate must be in (0, 1), got {baseline_rate}"
    );
    assert!(
        target_power > 0.0 && target_power < 1.0,
        "target_power must be in (0, 1), got {target_power}"
    );

    let mut lower = 1e-9;
    let mut upper = baseline_rate - 1e-9;
    #[allow(
        clippy::while_float,
        reason = "the bisection interval halves each iteration; convergence to 1e-10 is guaranteed"
    )]
    while upper - lower > 1e-10 {
        let mid = f64::midpoint(lower, upper);
        let meets_target =
            self_consistent_power(sample_size, baseline_rate, mid, confidence) >= target_power;
        (lower, upper) = if meets_target {
            (mid, upper)
        } else {
            (lower, mid)
        };
    }
    lower
}

/// Why a sizing design cannot be priced.
///
/// The domain restriction of companion §5.4.1 is reached by two different
/// routes, and they call for different corrective action. Carrying the
/// cause as data rather than only as prose lets a caller — a diagnostic, a
/// report, a conformance run — tell them apart without parsing a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizingRefusal {
    /// The baseline observed no successes, so its effective rate is exactly
    /// 0 at every sample size (companion §4.3.4) and no declared tolerance
    /// can sit below it. *Measure a baseline before sizing against it.*
    ZeroBaseline,
    /// The baseline is usable, but the declared tolerance does not sit
    /// strictly below it, so there is no degradation to detect.
    /// *Re-measure the baseline rather than raising the tolerance.*
    EmptyToleranceInterval,
}

impl SizingRefusal {
    /// The stable identifier the oracle's fixtures name this refusal by.
    #[must_use]
    pub const fn category(self) -> &'static str {
        match self {
            Self::ZeroBaseline => "ZERO_BASELINE",
            Self::EmptyToleranceInterval => "EMPTY_TOLERANCE_INTERVAL",
        }
    }

    /// The operator-facing explanation, naming the corrective action.
    #[must_use]
    pub fn message(self, baseline_rate: f64, minimum_acceptable_rate: f64) -> String {
        match self {
            Self::ZeroBaseline => "baseline_rate is exactly 0: the baseline observed no \
                 successes, so its effective rate is 0 at every sample size and there is no \
                 tolerated rate below it to detect. No sample size can price this design. \
                 Measure a baseline with at least one success before sizing against it."
                .to_owned(),
            Self::EmptyToleranceInterval => format!(
                "minimum_acceptable_rate ({minimum_acceptable_rate}) must sit below \
                 baseline_rate ({baseline_rate}): the tolerance declares how far below the \
                 measured baseline a true rate may drop; to demand more than the baseline \
                 delivered, re-measure the baseline rather than raising the tolerance"
            ),
        }
    }
}

/// Checks the sizing domain without panicking, naming the cause when it does
/// not hold.
///
/// The sizing entry points panic on an inadmissible design, which is the
/// right response to a misconfiguration discovered at pre-flight. This is
/// the same decision made available as a value, so a caller that wants to
/// ask before it commits — or to report the refusal rather than unwind —
/// can.
///
/// # Errors
///
/// Returns the [`SizingRefusal`] naming why the design cannot be priced.
///
/// # Panics
///
/// Panics if `baseline_rate` is outside `[0, 1)` or `minimum_acceptable_rate`
/// is not positive. Those are malformed inputs rather than inadmissible
/// designs: a refusal says a well-formed design cannot be priced, which is a
/// different statement from a rate that is not a rate.
pub fn check_sizing_domain(
    baseline_rate: f64,
    minimum_acceptable_rate: f64,
) -> Result<(), SizingRefusal> {
    if baseline_rate == 0.0 {
        return Err(SizingRefusal::ZeroBaseline);
    }
    assert!(
        baseline_rate > 0.0 && baseline_rate < 1.0,
        "baseline_rate must be in (0, 1), got {baseline_rate}"
    );
    assert!(
        minimum_acceptable_rate > 0.0,
        "minimum_acceptable_rate must be positive, got {minimum_acceptable_rate}"
    );
    if minimum_acceptable_rate >= baseline_rate {
        return Err(SizingRefusal::EmptyToleranceInterval);
    }
    Ok(())
}

/// Asserts the sizing domain: `baseline_rate` in (0, 1) and
/// `minimum_acceptable_rate` strictly below it (and above 0).
fn assert_sizing_domain(baseline_rate: f64, minimum_acceptable_rate: f64) {
    if let Err(refusal) = check_sizing_domain(baseline_rate, minimum_acceptable_rate) {
        panic!(
            "{}",
            refusal.message(baseline_rate, minimum_acceptable_rate)
        );
    }
}

#[cfg(test)]
#[allow(unused_must_use, reason = "test boilerplate may drop must_use values")]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn cl(v: f64) -> ConfidenceLevel {
        ConfidenceLevel::new(v)
    }

    // --- self_consistent_power ---

    #[test]
    fn power_increases_with_sample_size() {
        let sizes = [50, 150, 405, 1000];
        let powers: Vec<f64> = sizes
            .iter()
            .map(|&n| self_consistent_power(n, 0.96, 0.93, cl(0.95)))
            .collect();
        for pair in powers.windows(2) {
            assert!(pair[1] > pair[0], "power must increase with sample size");
        }
    }

    #[test]
    fn power_is_a_probability() {
        for n in [1, 10, 100, 5000] {
            let power = self_consistent_power(n, 0.9, 0.8, cl(0.95));
            assert!((0.0..=1.0).contains(&power));
        }
    }

    #[test]
    #[should_panic(expected = "sample_size must be positive")]
    fn power_panics_on_zero_sample_size() {
        self_consistent_power(0, 0.9, 0.8, cl(0.95));
    }

    #[test]
    #[should_panic(expected = "re-measure the baseline rather than raising the tolerance")]
    fn power_panics_when_tolerance_reaches_baseline() {
        self_consistent_power(100, 0.9, 0.9, cl(0.95));
    }

    #[test]
    #[should_panic(expected = "re-measure the baseline rather than raising the tolerance")]
    fn power_panics_when_tolerance_exceeds_baseline() {
        self_consistent_power(100, 0.9, 0.95, cl(0.95));
    }

    #[test]
    #[should_panic(expected = "baseline_rate must be in (0, 1)")]
    fn power_panics_on_perfect_baseline() {
        self_consistent_power(100, 1.0, 0.9, cl(0.95));
    }

    // --- required_sample_size ---

    #[test]
    fn required_sample_size_is_minimal() {
        let n = required_sample_size(0.87, 0.84, cl(0.95), 0.80);
        assert!(self_consistent_power(n, 0.87, 0.84, cl(0.95)) >= 0.80);
        assert!(self_consistent_power(n - 1, 0.87, 0.84, cl(0.95)) < 0.80);
    }

    #[test]
    fn tighter_tolerance_requires_more_samples() {
        let wide = required_sample_size(0.96, 0.90, cl(0.95), 0.80);
        let tight = required_sample_size(0.96, 0.93, cl(0.95), 0.80);
        assert!(tight > wide);
    }

    #[test]
    fn higher_target_power_requires_more_samples() {
        let modest = required_sample_size(0.96, 0.93, cl(0.95), 0.80);
        let demanding = required_sample_size(0.96, 0.93, cl(0.95), 0.90);
        assert!(demanding > modest);
    }

    #[test]
    #[should_panic(expected = "target_power must be in (0, 1)")]
    fn required_sample_size_panics_on_invalid_target_power() {
        required_sample_size(0.9, 0.8, cl(0.95), 1.0);
    }

    #[test]
    #[should_panic(expected = "re-measure the baseline rather than raising the tolerance")]
    fn required_sample_size_panics_when_tolerance_reaches_baseline() {
        required_sample_size(0.9, 0.9, cl(0.95), 0.80);
    }

    // --- detectable_rate ---

    #[test]
    fn detectable_rate_round_trips_through_required_sample_size() {
        // At exactly the required size, the detectable rate recovers the
        // declared tolerance (to the bisection's resolution as seen through
        // the discrete sample size).
        let n = required_sample_size(0.87, 0.84, cl(0.95), 0.80);
        let rate = detectable_rate(n, 0.87, cl(0.95), 0.80);
        assert_relative_eq!(rate, 0.84, epsilon = 1e-3);
        // The recovered rate is itself detectable at the target power.
        assert!(self_consistent_power(n, 0.87, rate, cl(0.95)) >= 0.80);
    }

    #[test]
    fn detectable_rate_rises_with_sample_size() {
        let coarse = detectable_rate(100, 0.87, cl(0.95), 0.80);
        let fine = detectable_rate(891, 0.87, cl(0.95), 0.80);
        assert!(fine > coarse, "more samples detect smaller degradations");
    }

    #[test]
    fn detectable_rate_stays_below_baseline() {
        let rate = detectable_rate(10_000, 0.87, cl(0.95), 0.80);
        assert!(rate < 0.87);
        assert!(rate > 0.0);
    }

    #[test]
    #[should_panic(expected = "sample_size must be positive")]
    fn detectable_rate_panics_on_zero_sample_size() {
        detectable_rate(0, 0.9, cl(0.95), 0.80);
    }

    #[test]
    #[should_panic(expected = "baseline_rate must be in (0, 1)")]
    fn detectable_rate_panics_on_invalid_baseline_rate() {
        detectable_rate(100, 1.0, cl(0.95), 0.80);
    }
}
