//! Latency distribution analysis using empirical percentiles.
//!
//! Computes nearest-rank percentiles, summary statistics, and threshold
//! derivation from baseline latency measurements. These are the statistical
//! building blocks for the latency testing dimension.

use statrs::distribution::{ContinuousCDF, Normal};

/// Returns the standard normal distribution N(0, 1).
///
/// # Panics
///
/// Cannot panic — parameters are compile-time constants.
fn standard_normal() -> Normal {
    Normal::new(0.0, 1.0).unwrap()
}

/// Computes the nearest-rank empirical percentile.
///
/// Uses the ceiling method: `index = ceil(p * n) - 1`, clamped to `[0, n-1]`.
/// This matches R's `quantile(type=1)` behaviour.
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
#[derive(Debug, Clone)]
pub struct LatencySummary {
    mean: f64,
    sd: f64,
    max: f64,
}

impl LatencySummary {
    /// Computes summary statistics from a latency sample.
    ///
    /// For a single observation, `sd` is `f64::NAN` (undefined with n-1 = 0
    /// denominator).
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

        let sd = if latencies.len() == 1 {
            f64::NAN
        } else {
            let sum_sq: f64 = latencies.iter().map(|x| (x - mean).powi(2)).sum();
            (sum_sq / (n - 1.0)).sqrt()
        };

        Self { mean, sd, max }
    }

    #[must_use]
    pub const fn mean(&self) -> f64 {
        self.mean
    }

    #[must_use]
    pub const fn sd(&self) -> f64 {
        self.sd
    }

    #[must_use]
    pub const fn max(&self) -> f64 {
        self.max
    }
}

/// Result of latency threshold derivation from baseline statistics.
#[derive(Debug, Clone)]
pub struct DerivedLatencyThreshold {
    raw_upper: f64,
    threshold: f64,
}

impl DerivedLatencyThreshold {
    #[must_use]
    pub const fn raw_upper(&self) -> f64 {
        self.raw_upper
    }

    #[must_use]
    pub const fn threshold(&self) -> f64 {
        self.threshold
    }
}

/// Derives a latency threshold from baseline statistics using a one-sided
/// upper confidence bound.
///
/// Formula:
/// ```text
/// se        = baseline_sd / sqrt(baseline_n)
/// raw_upper = baseline_percentile + z(confidence) * se
/// threshold = max(baseline_percentile, ceil(raw_upper))
/// ```
///
/// # Panics
///
/// - `baseline_n` must be positive.
/// - `confidence` must be in (0, 1).
#[must_use]
pub fn derive_latency_threshold(
    baseline_percentile: f64,
    baseline_sd: f64,
    baseline_n: u32,
    confidence: f64,
) -> DerivedLatencyThreshold {
    assert!(baseline_n > 0, "baseline_n must be positive");
    assert!(
        confidence > 0.0 && confidence < 1.0,
        "confidence must be in (0, 1), got {confidence}"
    );

    let alpha = 1.0 - confidence;
    let z = standard_normal().inverse_cdf(1.0 - alpha);
    let se = baseline_sd / (f64::from(baseline_n)).sqrt();
    let raw_upper = baseline_percentile + z * se;
    let threshold = raw_upper.ceil().max(baseline_percentile);

    DerivedLatencyThreshold {
        raw_upper,
        threshold,
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
    fn summary_single_observation_sd_is_nan() {
        let s = LatencySummary::from_latencies(&[250.0]);
        assert_relative_eq!(s.mean(), 250.0);
        assert!(s.sd().is_nan());
        assert_relative_eq!(s.max(), 250.0);
    }

    #[test]
    fn summary_identical_values_sd_is_zero() {
        let data = vec![150.0; 10];
        let s = LatencySummary::from_latencies(&data);
        assert_relative_eq!(s.mean(), 150.0);
        assert_relative_eq!(s.sd(), 0.0);
        assert_relative_eq!(s.max(), 150.0);
    }

    #[test]
    fn threshold_zero_variance_equals_baseline() {
        let t = derive_latency_threshold(200.0, 0.0, 100, 0.95);
        assert_relative_eq!(t.raw_upper(), 200.0);
        assert_relative_eq!(t.threshold(), 200.0);
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
