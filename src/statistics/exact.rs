//! The exact-boundary convention shared by every exact decision rule
//! (Statistical Companion §10.6).
//!
//! Each exact rule compares a probability with alpha by an inclusive rule: a
//! Fisher p-value at most alpha fails a test count, a binomial upper tail at
//! most alpha admits a count, a precedence breach probability at most alpha
//! admits a rank. At an exact boundary the probability *equals* alpha, and
//! double-precision evaluation can land on either side of it. The
//! convention:
//!
//! 1. compute the probability in double precision;
//! 2. when `|value − alpha| ≤ BOUNDARY_GUARD · alpha`, recompute it exactly,
//!    in rational arithmetic, from the declared inputs;
//! 3. apply the inclusive rule to the exact value.
//!
//! Declared rates and levels are read as the exact decimals they are written
//! as: `0.05` is `1/20`, `0.995` is `199/200`.

use num_bigint::BigUint;

/// Relative band around alpha inside which a probability is recomputed
/// exactly.
pub const BOUNDARY_GUARD: f64 = 1e-9;

/// A non-negative rational number, compared exactly.
#[derive(Debug, Clone)]
pub(crate) struct Rational {
    numerator: BigUint,
    denominator: BigUint,
}

impl Rational {
    /// A rational with a positive denominator.
    fn new(numerator: BigUint, denominator: BigUint) -> Self {
        assert!(
            denominator != BigUint::ZERO,
            "a rational needs a positive denominator"
        );
        Self {
            numerator,
            denominator,
        }
    }

    /// The numerator and the denominator.
    pub(crate) const fn parts(&self) -> (&BigUint, &BigUint) {
        (&self.numerator, &self.denominator)
    }

    /// Whether `self ≤ other`, by cross-multiplication.
    fn at_most(&self, other: &Self) -> bool {
        &self.numerator * &other.denominator <= &other.numerator * &self.denominator
    }
}

/// A declared decimal as the exact rational it was written as.
///
/// Fifteen significant digits recover what was typed from the nearest
/// double (`0.05` becomes `1/20`, not its binary neighbour).
///
/// # Panics
///
/// Panics if `value` is negative or not finite.
#[must_use]
pub(crate) fn exact_decimal(value: f64) -> Rational {
    assert!(
        value.is_finite() && value >= 0.0,
        "a declared decimal must be finite and non-negative, got {value}"
    );
    let scientific = format!("{value:.14e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("scientific formatting always carries an exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().expect("the exponent is an integer");
    let numerator: BigUint = digits.parse().expect("the mantissa digits form an integer");
    // The mantissa carries fourteen digits after its point.
    let scale = exponent - 14;
    let ten = BigUint::from(10_u32);
    if scale >= 0 {
        Rational::new(
            numerator * ten.pow(scale.unsigned_abs()),
            BigUint::from(1_u32),
        )
    } else {
        Rational::new(numerator, ten.pow(scale.unsigned_abs()))
    }
}

/// Whether `value ≤ alpha` under the exact-boundary convention.
///
/// `exact` computes the probability in rational arithmetic; it is called
/// only inside the guard band.
#[must_use]
pub(crate) fn at_most_alpha(value: f64, alpha: f64, exact: impl FnOnce() -> Rational) -> bool {
    if (value - alpha).abs() <= BOUNDARY_GUARD * alpha {
        return exact().at_most(&exact_decimal(alpha));
    }
    value <= alpha
}

/// `C(n, k)` exactly.
fn choose(n: u64, k: u64) -> BigUint {
    if k > n {
        return BigUint::ZERO;
    }
    let k = k.min(n - k);
    (0..k).fold(BigUint::from(1_u32), |acc, i| acc * (n - i) / (i + 1))
}

/// The one-sided Fisher p-value `P(X ≤ k_t)` as a rational.
///
/// `X` is hypergeometric: of `s = k_b + k_t` pooled successes, the number
/// falling in the test's `n_t` of the `n_b + n_t` trials.
#[must_use]
pub(crate) fn fisher_p_value_exact(
    test_successes: u32,
    baseline_successes: u32,
    baseline_trials: u32,
    test_samples: u32,
) -> Rational {
    let total = u64::from(baseline_trials) + u64::from(test_samples);
    let pooled = u64::from(baseline_successes) + u64::from(test_successes);
    let draws = u64::from(test_samples);
    let low = pooled.saturating_sub(u64::from(baseline_trials));
    let numerator = (low..=u64::from(test_successes)).fold(BigUint::ZERO, |sum, x| {
        sum + choose(pooled, x) * choose(total - pooled, draws - x)
    });
    Rational::new(numerator, choose(total, draws))
}

/// `P(K ≥ count)` for `K ~ Bin(trials, rate)` as a rational, the rate read
/// as the exact decimal it was declared as.
#[must_use]
pub(crate) fn binomial_upper_tail_exact(count: u32, trials: u32, rate: f64) -> Rational {
    if count == 0 {
        return Rational::new(BigUint::from(1_u32), BigUint::from(1_u32));
    }
    if count > trials {
        return Rational::new(BigUint::ZERO, BigUint::from(1_u32));
    }
    let q = exact_decimal(rate);
    let (a, b) = (q.numerator, q.denominator);
    let complement = &b - &a;
    let numerator = (count..=trials).fold(BigUint::ZERO, |sum, j| {
        sum + choose(u64::from(trials), u64::from(j)) * a.pow(j) * complement.pow(trials - j)
    });
    Rational::new(numerator, b.pow(trials))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ratio(numerator: u64, denominator: u64) -> Rational {
        Rational::new(BigUint::from(numerator), BigUint::from(denominator))
    }

    fn equal(left: &Rational, right: &Rational) -> bool {
        left.at_most(right) && right.at_most(left)
    }

    #[test]
    fn declared_decimals_are_read_as_written() {
        assert!(equal(&exact_decimal(0.05), &ratio(1, 20)));
        assert!(equal(&exact_decimal(0.995), &ratio(199, 200)));
        assert!(equal(&exact_decimal(0.003_125), &ratio(1, 320)));
        assert!(equal(&exact_decimal(3.0), &ratio(3, 1)));
    }

    #[test]
    fn outside_the_guard_band_the_double_decides() {
        let never = || -> Rational { panic!("the exact value must not be computed") };
        assert!(at_most_alpha(0.04, 0.05, never));
        assert!(!at_most_alpha(0.06, 0.05, never));
    }

    #[test]
    fn inside_the_guard_band_the_exact_value_decides() {
        // A double a hair above alpha whose exact value equals it: inclusive.
        let just_above = 0.05 * (1.0 + 1e-12);
        assert!(at_most_alpha(just_above, 0.05, || ratio(1, 20)));
        // A double a hair below alpha whose exact value exceeds it.
        let just_below = 0.05 * (1.0 - 1e-12);
        assert!(!at_most_alpha(just_below, 0.05, || ratio(
            1_000_001, 20_000_000
        )));
    }

    #[test]
    fn fisher_exact_p_value_is_a_hypergeometric_tail() {
        // 12 of 12 baseline, 2 of 4 test: P(X <= 2) with s = 14 of 16, 4 drawn.
        // Support starts at 2: C(14,2) C(2,2) / C(16,4) = 91 / 1820.
        let exact = fisher_p_value_exact(2, 12, 12, 4);
        assert!(equal(&exact, &ratio(91, 1820)));
    }

    #[test]
    fn binomial_exact_upper_tail_matches_the_closed_form() {
        // P(K >= 5) for K ~ Bin(5, 0.5) = 1/32.
        assert!(equal(&binomial_upper_tail_exact(5, 5, 0.5), &ratio(1, 32)));
        assert!(equal(&binomial_upper_tail_exact(0, 5, 0.5), &ratio(1, 1)));
        assert!(equal(&binomial_upper_tail_exact(6, 5, 0.5), &ratio(0, 1)));
    }
}
