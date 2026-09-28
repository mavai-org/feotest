//! Accurate discrete distribution primitives for the exact decision rules.
//!
//! The binomial probability mass uses Loader's saddle-point form (the
//! Stirling-series remainder and the deviance term `bd0`), which keeps full
//! relative precision far into the tails where a difference of log-gamma
//! values would lose digits. The hypergeometric lower tail is the mass at
//! its end point times the ratio sum of the preceding terms — the method of
//! the reference oracle's `phyper`. Binomial tails and the regularised
//! incomplete beta come from `statrs`; the beta quantile is found by
//! bisection on it, so it carries the incomplete beta's own precision.

use statrs::distribution::{Binomial, DiscreteCDF};
use statrs::function::beta::beta_reg;
use statrs::function::gamma::ln_gamma;

/// `ln(2π)`.
const LN_2PI: f64 = 1.837_877_066_409_345_5;

/// `ln(√(2π))`.
const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_8;

/// The error of Stirling's approximation to `ln(n!)`:
/// `ln(n!) − (n + ½) ln(n) + n − ln(√(2π))`.
///
/// Below 16 it is computed from `ln Γ` directly (the terms are small and
/// the difference keeps twelve significant digits); above, from the
/// asymptotic series, which is exact to double precision there.
fn stirling_error(n: f64) -> f64 {
    const S0: f64 = 1.0 / 12.0;
    const S1: f64 = 1.0 / 360.0;
    const S2: f64 = 1.0 / 1260.0;
    const S3: f64 = 1.0 / 1680.0;
    const S4: f64 = 1.0 / 1188.0;
    if n <= 15.0 {
        return (n + 0.5).mul_add(-n.ln(), ln_gamma(n + 1.0)) + n - LN_SQRT_2PI;
    }
    let nn = n * n;
    if n > 500.0 {
        (S0 - S1 / nn) / n
    } else if n > 80.0 {
        (S0 - (S1 - S2 / nn) / nn) / n
    } else if n > 35.0 {
        (S0 - (S1 - (S2 - S3 / nn) / nn) / nn) / n
    } else {
        (S0 - (S1 - (S2 - (S3 - S4 / nn) / nn) / nn) / nn) / n
    }
}

/// The deviance term `x ln(x / np) + np − x`, computed without
/// cancellation when `x` is close to `np`.
fn deviance(x: f64, np: f64) -> f64 {
    if (x - np).abs() < 0.1 * (x + np) {
        let mut v = (x - np) / (x + np);
        let mut sum = (x - np) * v;
        let mut ej = 2.0 * x * v;
        v *= v;
        let mut j = 1.0_f64;
        loop {
            ej *= v;
            let next = sum + ej / (2.0f64.mul_add(j, 1.0));
            if next.to_bits() == sum.to_bits() {
                return next;
            }
            sum = next;
            j += 1.0;
        }
    }
    x.mul_add((x / np).ln(), np - x)
}

/// The binomial probability mass `P(K = x)` for `K ~ Bin(n, p)`, with
/// `q = 1 − p` supplied so a complement never loses digits.
fn binomial_mass_raw(x: f64, n: f64, p: f64, q: f64) -> f64 {
    if p == 0.0 {
        return if x == 0.0 { 1.0 } else { 0.0 };
    }
    if q == 0.0 {
        return if x.to_bits() == n.to_bits() { 1.0 } else { 0.0 };
    }
    if x == 0.0 {
        if n == 0.0 {
            return 1.0;
        }
        let log_mass = if p < 0.1 {
            n.mul_add(-p, -deviance(n, n * q))
        } else {
            n * q.ln()
        };
        return log_mass.exp();
    }
    if x.to_bits() == n.to_bits() {
        let log_mass = if q < 0.1 {
            n.mul_add(-q, -deviance(n, n * p))
        } else {
            n * p.ln()
        };
        return log_mass.exp();
    }
    let log_core = stirling_error(n)
        - stirling_error(x)
        - stirling_error(n - x)
        - deviance(x, n * p)
        - deviance(n - x, n * q);
    let log_factor = LN_2PI + x.ln() + (-x / n).ln_1p();
    (0.5f64.mul_add(-log_factor, log_core)).exp()
}

/// The binomial probability mass `P(K = successes)`, `K ~ Bin(trials, rate)`.
///
/// # Panics
///
/// Panics if `rate` is outside `[0, 1]`.
#[must_use]
pub(super) fn binomial_mass(successes: u32, trials: u32, rate: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&rate),
        "rate must be in [0, 1], got {rate}"
    );
    if successes > trials {
        return 0.0;
    }
    binomial_mass_raw(f64::from(successes), f64::from(trials), rate, 1.0 - rate)
}

/// `P(K ≤ count)` for `K ~ Bin(trials, rate)`; 0 for a negative count.
///
/// # Panics
///
/// Panics if `rate` is outside `[0, 1]`.
#[must_use]
pub(super) fn binomial_cdf(count: i64, trials: u32, rate: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&rate),
        "rate must be in [0, 1], got {rate}"
    );
    if count < 0 {
        return 0.0;
    }
    if count >= i64::from(trials) {
        return 1.0;
    }
    binomial(trials, rate).cdf(count.unsigned_abs())
}

/// `P(K ≥ count)` for `K ~ Bin(trials, rate)`; 1 for a count of 0 or less.
///
/// # Panics
///
/// Panics if `rate` is outside `[0, 1]`.
#[must_use]
pub(super) fn binomial_upper_tail(count: i64, trials: u32, rate: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&rate),
        "rate must be in [0, 1], got {rate}"
    );
    if count <= 0 {
        return 1.0;
    }
    if count > i64::from(trials) {
        return 0.0;
    }
    binomial(trials, rate).sf(count.unsigned_abs() - 1)
}

/// The binomial distribution, whose parameters every caller has validated.
fn binomial(trials: u32, rate: f64) -> Binomial {
    Binomial::new(rate, u64::from(trials)).expect("rate is validated to lie in [0, 1]")
}

/// The smallest count `k` with `P(K ≤ k) ≥ probability`, `K ~ Bin(trials, rate)`.
///
/// # Panics
///
/// Panics if `rate` or `probability` is outside `[0, 1]`.
#[must_use]
pub(super) fn binomial_quantile(probability: f64, trials: u32, rate: f64) -> u32 {
    assert!(
        (0.0..=1.0).contains(&probability),
        "probability must be in [0, 1], got {probability}"
    );
    let (mut low, mut high) = (0_u32, trials);
    while low < high {
        let mid = low + (high - low) / 2;
        if binomial_cdf(i64::from(mid), trials, rate) >= probability {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

/// The hypergeometric lower tail `P(X ≤ x)`, where `X` counts the marked
/// items among `draws` drawn without replacement from `marked + unmarked`.
///
/// # Panics
///
/// Panics if `draws` exceeds the population.
#[must_use]
pub(super) fn hypergeometric_cdf(x: i64, marked: u32, unmarked: u32, draws: u32) -> f64 {
    assert!(
        u64::from(draws) <= u64::from(marked) + u64::from(unmarked),
        "draws ({draws}) must not exceed the population ({marked} + {unmarked})"
    );
    let (mut x, mut marked, mut unmarked) = (x, i64::from(marked), i64::from(unmarked));
    let draws = i64::from(draws);
    // Sum over the shorter tail; the other is its complement.
    let lower_tail = x * (marked + unmarked) <= draws * marked;
    if !lower_tail {
        std::mem::swap(&mut marked, &mut unmarked);
        x = draws - x - 1;
    }
    let tail = if x < 0 || x < draws - unmarked {
        0.0
    } else if x >= marked || x >= draws {
        1.0
    } else {
        hypergeometric_mass(x, marked, unmarked, draws)
            * hypergeometric_ratio_sum(x, marked, unmarked, draws)
    };
    if lower_tail { tail } else { 0.5 - tail + 0.5 }
}

/// `P(X = x)` for the hypergeometric distribution, as a ratio of binomial
/// masses at the sampling fraction (each accurate in its own tail).
#[allow(
    clippy::cast_precision_loss,
    reason = "counts are u32-bounded and exact in f64"
)]
fn hypergeometric_mass(x: i64, marked: i64, unmarked: i64, draws: i64) -> f64 {
    let population = (marked + unmarked) as f64;
    let p = draws as f64 / population;
    let q = (marked + unmarked - draws) as f64 / population;
    let marked_mass = binomial_mass_raw(x as f64, marked as f64, p, q);
    let unmarked_mass = binomial_mass_raw((draws - x) as f64, unmarked as f64, p, q);
    let whole_mass = binomial_mass_raw(draws as f64, population, p, q);
    marked_mass * unmarked_mass / whole_mass
}

/// `Σ_{i ≤ x} P(X = i) / P(X = x)`, summed downward until the terms no
/// longer change the sum.
#[allow(
    clippy::cast_precision_loss,
    reason = "counts are u32-bounded and exact in f64"
)]
fn hypergeometric_ratio_sum(x: i64, marked: i64, unmarked: i64, draws: i64) -> f64 {
    let (marked, unmarked, draws) = (marked as f64, unmarked as f64, draws as f64);
    let mut i = x as f64;
    let mut sum = 0.0;
    let mut term = 1.0;
    while i > 0.0 && term >= f64::EPSILON * sum {
        term *= i * (unmarked - draws + i) / (draws + 1.0 - i) / (marked + 1.0 - i);
        sum += term;
        i -= 1.0;
    }
    1.0 + sum
}

/// Bisects `[low, high]` down to `tolerance`: where `lowers_high(mid)` holds
/// the upper end moves down to `mid`, otherwise the lower end moves up.
/// Returns the final bracket.
pub(super) fn bisect(
    mut low: f64,
    mut high: f64,
    tolerance: f64,
    lowers_high: impl Fn(f64) -> bool,
) -> (f64, f64) {
    loop {
        if high - low <= tolerance {
            return (low, high);
        }
        let mid = f64::midpoint(low, high);
        if lowers_high(mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
}

/// The regularised incomplete beta `I_x(a, b)`.
#[must_use]
pub(super) fn incomplete_beta(x: f64, a: f64, b: f64) -> f64 {
    beta_reg(a, b, x)
}

/// The quantile of `Beta(a, b)` at `probability`, by bisection on the
/// regularised incomplete beta to double precision.
///
/// # Panics
///
/// Panics if `probability` is outside `[0, 1]` or a shape is not positive.
#[must_use]
pub(super) fn beta_quantile(probability: f64, a: f64, b: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&probability),
        "probability must be in [0, 1], got {probability}"
    );
    assert!(a > 0.0 && b > 0.0, "beta shapes must be positive");
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..200 {
        let mid = f64::midpoint(low, high);
        if mid <= low || mid >= high {
            break;
        }
        if incomplete_beta(mid, a, b) < probability {
            low = mid;
        } else {
            high = mid;
        }
    }
    f64::midpoint(low, high)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn binomial_mass_matches_the_closed_form_for_small_counts() {
        // C(10, 3) 0.3^3 0.7^7
        let expected = 120.0 * 0.3_f64.powi(3) * 0.7_f64.powi(7);
        assert_relative_eq!(binomial_mass(3, 10, 0.3), expected, max_relative = 1e-12);
    }

    #[test]
    fn binomial_mass_handles_the_end_points() {
        assert_relative_eq!(binomial_mass(0, 5, 0.5), 1.0 / 32.0, max_relative = 1e-14);
        assert_relative_eq!(binomial_mass(5, 5, 0.5), 1.0 / 32.0, max_relative = 1e-14);
        assert_relative_eq!(binomial_mass(0, 5, 0.0), 1.0);
        assert_relative_eq!(binomial_mass(5, 5, 1.0), 1.0);
        assert_relative_eq!(binomial_mass(4, 5, 1.0), 0.0);
    }

    #[test]
    fn binomial_masses_sum_to_one_at_large_n() {
        let total: f64 = (0..=2000).map(|k| binomial_mass(k, 2000, 0.951)).sum();
        assert_relative_eq!(total, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn binomial_tails_are_complements() {
        let below = binomial_cdf(90, 100, 0.95);
        let above = binomial_upper_tail(91, 100, 0.95);
        assert_relative_eq!(below + above, 1.0, epsilon = 1e-14);
        assert_relative_eq!(binomial_cdf(-1, 100, 0.95), 0.0);
        assert_relative_eq!(binomial_upper_tail(0, 100, 0.95), 1.0);
        assert_relative_eq!(binomial_upper_tail(101, 100, 0.95), 0.0);
    }

    #[test]
    fn binomial_quantile_is_the_smallest_count_reaching_the_probability() {
        let k = binomial_quantile(0.5, 10, 0.5);
        assert_eq!(k, 5);
        assert!(binomial_cdf(i64::from(k) - 1, 10, 0.5) < 0.5);
    }

    #[test]
    fn hypergeometric_cdf_matches_a_direct_sum() {
        // 20 marked, 30 unmarked, 10 drawn: P(X <= 3) by direct summation.
        let choose = |n: u32, k: u32| -> f64 {
            (0..k).fold(1.0, |acc, i| acc * f64::from(n - i) / f64::from(i + 1))
        };
        let total = choose(50, 10);
        let direct: f64 = (0..=3)
            .map(|x| choose(20, x) * choose(30, 10 - x) / total)
            .sum();
        assert_relative_eq!(
            hypergeometric_cdf(3, 20, 30, 10),
            direct,
            max_relative = 1e-12
        );
        // The upper-tail route (x above the mean) agrees too.
        let direct_high: f64 = (0..=7)
            .map(|x| choose(20, x) * choose(30, 10 - x) / total)
            .sum();
        assert_relative_eq!(
            hypergeometric_cdf(7, 20, 30, 10),
            direct_high,
            max_relative = 1e-12
        );
    }

    #[test]
    fn hypergeometric_cdf_is_zero_below_and_one_above_the_support() {
        assert_relative_eq!(hypergeometric_cdf(-1, 20, 30, 10), 0.0);
        assert_relative_eq!(hypergeometric_cdf(10, 20, 30, 10), 1.0);
        // With 45 marked of 50 and 10 drawn, at least 5 are marked.
        assert_relative_eq!(hypergeometric_cdf(4, 45, 5, 10), 0.0);
    }

    #[test]
    fn stirling_error_is_continuous_across_the_series_switch() {
        let below = stirling_error(15.0);
        let above = stirling_error(16.0);
        let direct = |n: f64| (n + 0.5).mul_add(-n.ln(), ln_gamma(n + 1.0)) + n - LN_SQRT_2PI;
        assert_relative_eq!(below, direct(15.0), max_relative = 1e-12);
        assert_relative_eq!(above, direct(16.0), max_relative = 1e-9);
    }

    #[test]
    fn bisect_narrows_to_the_crossing() {
        let (low, high) = bisect(0.0, 1.0, 1e-12, |x| x >= 0.3);
        assert!(low < 0.3 && high >= 0.3);
        assert!(high - low <= 1e-12);
    }

    #[test]
    fn beta_quantile_inverts_the_incomplete_beta() {
        let x = beta_quantile(0.05, 91.0, 10.0);
        assert_relative_eq!(incomplete_beta(x, 91.0, 10.0), 0.05, epsilon = 1e-14);
    }
}
