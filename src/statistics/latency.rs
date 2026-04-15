//! Latency distribution analysis using empirical percentiles.
//!
//! Computes nearest-rank percentiles, summary statistics, and threshold
//! derivation from baseline latency measurements. The statistical model is
//! specified by the javai-R oracle (`R/latency.R`) and is non-parametric:
//! thresholds are the k-th order statistic of a baseline sample, where k is
//! the exact binomial upper bound for the requested percentile.

use statrs::distribution::{Binomial, DiscreteCDF};

/// Computes the nearest-rank empirical percentile.
///
/// Uses the ceiling method: `index = ceil(p * n) - 1`, clamped to `[0, n-1]`.
/// Matches R's `quantile(type = 1)` behaviour.
///
/// # Panics
///
/// - `latencies` must not be empty.
/// - `percentile` must be in (0, 1].
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn nearest_rank_percentile(latencies: &[f64], percentile: f64) -> f64 {
    assert!(!latencies.is_empty(), "latencies must not be empty");
    assert!(
        percentile > 0.0 && percentile <= 1.0,
        "percentile must be in (0, 1], got {percentile}"
    );

    let mut sorted = latencies.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("latencies must not contain NaN"));

    let n = sorted.len();
    let raw_index = (percentile * n as f64).ceil() as usize;
    let index = raw_index.saturating_sub(1).min(n - 1);

    sorted[index]
}

/// Summary statistics for a latency sample.
///
/// Reports sample mean and maximum only. Standard deviation is deliberately
/// omitted: the threshold derivation is non-parametric and does not use it,
/// and sd is not a well-behaved summary for the distributions of interest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencySummary {
    mean: f64,
    max: f64,
}

impl LatencySummary {
    /// Computes summary statistics from a latency sample.
    ///
    /// # Panics
    ///
    /// `latencies` must not be empty.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn from_latencies(latencies: &[f64]) -> Self {
        assert!(!latencies.is_empty(), "latencies must not be empty");

        let n = latencies.len() as f64;
        let mean = latencies.iter().sum::<f64>() / n;
        let max = latencies.iter().copied().fold(f64::NEG_INFINITY, f64::max);

        Self { mean, max }
    }

    /// Sample arithmetic mean.
    #[must_use]
    pub const fn mean(&self) -> f64 {
        self.mean
    }

    /// Maximum observed latency.
    #[must_use]
    pub const fn max(&self) -> f64 {
        self.max
    }
}

/// Result of latency threshold derivation from a baseline sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DerivedLatencyThreshold {
    rank: u32,
    threshold: f64,
    baseline_percentile: f64,
    n: u32,
}

impl DerivedLatencyThreshold {
    /// The 1-indexed rank `k` of the threshold within the sorted baseline.
    #[must_use]
    pub const fn rank(&self) -> u32 {
        self.rank
    }

    /// The threshold value: the `k`-th order statistic of the baseline.
    #[must_use]
    pub const fn threshold(&self) -> f64 {
        self.threshold
    }

    /// The nearest-rank point estimate of the baseline percentile.
    #[must_use]
    pub const fn baseline_percentile(&self) -> f64 {
        self.baseline_percentile
    }

    /// Baseline sample size.
    #[must_use]
    pub const fn n(&self) -> u32 {
        self.n
    }
}

/// Derives a latency threshold from a baseline sample using the exact
/// binomial order-statistic upper bound.
///
/// Given a baseline of `n_s` successful-response latencies, a target
/// percentile `p`, and a one-sided confidence level `c`, the threshold is
/// the `k`-th order statistic of the sorted baseline, where
/// `k = qbinom(1 - alpha, n_s, p) + 1`, clamped to `[ceil(p * n_s), n_s]`.
///
/// The construction is exact for any continuous underlying distribution and
/// requires no parametric assumption. The returned threshold is always an
/// observed latency.
///
/// # Panics
///
/// - `baseline_latencies` must not be empty.
/// - `percentile` must be in (0, 1].
/// - `confidence` must be in (0, 1).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn derive_latency_threshold(
    baseline_latencies: &[f64],
    percentile: f64,
    confidence: f64,
) -> DerivedLatencyThreshold {
    assert!(
        !baseline_latencies.is_empty(),
        "baseline_latencies must not be empty"
    );
    assert!(
        percentile > 0.0 && percentile <= 1.0,
        "percentile must be in (0, 1], got {percentile}"
    );
    assert!(
        confidence > 0.0 && confidence < 1.0,
        "confidence must be in (0, 1), got {confidence}"
    );

    let mut sorted = baseline_latencies.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("latencies must not contain NaN"));
    let n = sorted.len();
    let n_u64 = n as u64;

    // qbinom(1 - alpha, n, p) — smallest integer k' such that P(X <= k') >= 1 - alpha.
    // Clamp confidence to avoid the Binomial::inverse_cdf panic on p == 0 or 1.
    let alpha = 1.0 - confidence;
    let dist = Binomial::new(percentile, n_u64).expect("valid binomial parameters");
    let q = dist.inverse_cdf(1.0 - alpha);

    // k = qbinom(...) + 1, clamped to [ceil(p * n_s), n_s].
    #[allow(clippy::cast_possible_wrap)]
    let raw_rank = q.saturating_add(1);
    let point_rank = (percentile * n as f64).ceil() as u64;
    let clamped = raw_rank.max(point_rank).min(n_u64);
    let k = clamped.max(1) as usize;

    let threshold = sorted[k - 1];
    let baseline_percentile = {
        let point_index = (percentile * n as f64).ceil() as usize;
        let idx = point_index.saturating_sub(1).min(n - 1);
        sorted[idx]
    };

    DerivedLatencyThreshold {
        rank: clamped as u32,
        threshold,
        baseline_percentile,
        n: n as u32,
    }
}

/// Minimum successful-sample count required for a percentile estimate to be
/// non-degenerate.
///
/// Matches `latency_min_samples` in `javai-R/R/latency.R`:
/// p ≤ 0.50 → 5; p ≤ 0.90 → 10; p ≤ 0.95 → 20; p ≤ 0.99 → 100; else 100.
#[must_use]
pub const fn min_samples_for(percentile: f64) -> u32 {
    if percentile <= 0.50 {
        5
    } else if percentile <= 0.90 {
        10
    } else if percentile <= 0.95 {
        20
    } else {
        100
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn percentile_of_single_value() {
        assert_relative_eq!(nearest_rank_percentile(&[42.0], 0.5), 42.0);
        assert_relative_eq!(nearest_rank_percentile(&[42.0], 0.99), 42.0);
    }

    #[test]
    fn percentile_sorts_unsorted_input() {
        let data = vec![300.0, 100.0, 200.0];
        assert_relative_eq!(nearest_rank_percentile(&data, 0.5), 200.0);
    }

    #[test]
    fn percentile_p_one_returns_maximum() {
        let data = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        assert_relative_eq!(nearest_rank_percentile(&data, 1.0), 500.0);
    }

    #[test]
    fn summary_mean_and_max() {
        let s = LatencySummary::from_latencies(&[100.0, 200.0, 300.0, 400.0, 500.0]);
        assert_relative_eq!(s.mean(), 300.0);
        assert_relative_eq!(s.max(), 500.0);
    }

    #[test]
    fn summary_identical_values() {
        let s = LatencySummary::from_latencies(&[150.0; 10]);
        assert_relative_eq!(s.mean(), 150.0);
        assert_relative_eq!(s.max(), 150.0);
    }

    #[test]
    fn threshold_identical_baseline_collapses_to_common_value() {
        let data = vec![150.0; 100];
        let t = derive_latency_threshold(&data, 0.95, 0.95);
        assert_relative_eq!(t.threshold(), 150.0);
        assert_relative_eq!(t.baseline_percentile(), 150.0);
    }

    #[test]
    fn threshold_rank_never_below_point_estimate() {
        let data: Vec<f64> = (1..=100).map(f64::from).collect();
        let t = derive_latency_threshold(&data, 0.95, 0.95);
        assert!(t.rank() >= 95, "rank {} should be >= ceil(0.95 * 100)", t.rank());
        assert!(t.threshold() >= t.baseline_percentile());
    }

    #[test]
    fn threshold_saturates_at_n_for_tight_bounds() {
        // Small n_s vs. high p: rank saturates at n and threshold = max.
        let data = vec![100.0, 120.0, 140.0, 160.0, 180.0, 200.0, 250.0, 300.0, 400.0, 500.0];
        let t = derive_latency_threshold(&data, 0.99, 0.95);
        assert_eq!(t.rank(), 10);
        assert_relative_eq!(t.threshold(), 500.0);
    }

    #[test]
    fn higher_confidence_yields_monotone_threshold() {
        let data: Vec<f64> = (1..=500).map(f64::from).collect();
        let t90 = derive_latency_threshold(&data, 0.95, 0.90);
        let t95 = derive_latency_threshold(&data, 0.95, 0.95);
        let t99 = derive_latency_threshold(&data, 0.95, 0.99);
        assert!(t90.threshold() <= t95.threshold());
        assert!(t95.threshold() <= t99.threshold());
    }

    #[test]
    fn min_samples_matches_reference() {
        assert_eq!(min_samples_for(0.50), 5);
        assert_eq!(min_samples_for(0.90), 10);
        assert_eq!(min_samples_for(0.95), 20);
        assert_eq!(min_samples_for(0.99), 100);
    }

    #[test]
    #[should_panic(expected = "latencies must not be empty")]
    fn percentile_rejects_empty_slice() {
        let _ = nearest_rank_percentile(&[], 0.5);
    }

    #[test]
    #[should_panic(expected = "percentile must be in (0, 1]")]
    fn percentile_rejects_zero() {
        let _ = nearest_rank_percentile(&[1.0], 0.0);
    }
}
