//! The single latency criterion a contract may declare.

use std::time::Duration;

use crate::latency::percentile::Percentile;
use crate::latency::thresholds::LatencyThresholds;

/// A contract's latency commitment.
///
/// A contract declares **at most one** latency criterion — a service has a
/// single latency profile, so a single criterion holds the whole commitment.
/// That one criterion may still bound several percentiles
/// (`p95`, `p99`, …), each with its own ceiling.
///
/// Built from [`meeting`](Self::meeting), accumulating ceilings with
/// [`at_most`](Self::at_most):
///
/// ```
/// use feotest::latency::{LatencyCriterion, Percentile};
/// use std::time::Duration;
///
/// let latency = LatencyCriterion::meeting()
///     .at_most(Percentile::P95, Duration::from_millis(500))
///     .at_most(Percentile::P99, Duration::from_millis(1500));
///
/// assert_eq!(latency.thresholds().get(Percentile::P95), Some(Duration::from_millis(500)));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyCriterion {
    thresholds: LatencyThresholds,
}

impl LatencyCriterion {
    /// Begins a latency criterion whose ceilings are normative targets the
    /// service is required to meet. Each declared percentile is enforced
    /// strictly: an observed percentile above its ceiling fails the criterion.
    ///
    /// Chain [`at_most`](Self::at_most) to bound one or more percentiles.
    #[must_use]
    pub const fn meeting() -> Self {
        Self {
            thresholds: LatencyThresholds::new(),
        }
    }

    /// Bounds a percentile at `max`. The observed percentile must not exceed it.
    ///
    /// Re-declaring a percentile replaces its earlier ceiling.
    ///
    /// # Panics
    ///
    /// Panics if `max` is zero — a zero ceiling is meaningless.
    #[must_use]
    pub fn at_most(mut self, percentile: Percentile, max: Duration) -> Self {
        self.thresholds = self.thresholds.with(percentile, max);
        self
    }

    /// The percentile ceilings declared on this criterion.
    #[must_use]
    pub const fn thresholds(&self) -> &LatencyThresholds {
        &self.thresholds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_starts_with_no_ceilings() {
        assert!(LatencyCriterion::meeting().thresholds().is_empty());
    }

    #[test]
    fn at_most_records_a_ceiling_per_percentile() {
        let latency = LatencyCriterion::meeting()
            .at_most(Percentile::P95, Duration::from_millis(500))
            .at_most(Percentile::P99, Duration::from_millis(1500));

        assert_eq!(
            latency.thresholds().get(Percentile::P95),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            latency.thresholds().get(Percentile::P99),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(latency.thresholds().get(Percentile::P50), None);
    }

    #[test]
    fn declaration_order_does_not_matter() {
        let p95_first = LatencyCriterion::meeting()
            .at_most(Percentile::P95, Duration::from_millis(500))
            .at_most(Percentile::P99, Duration::from_millis(1500));
        let p99_first = LatencyCriterion::meeting()
            .at_most(Percentile::P99, Duration::from_millis(1500))
            .at_most(Percentile::P95, Duration::from_millis(500));

        assert_eq!(p95_first, p99_first);
    }

    #[test]
    fn re_declaring_a_percentile_replaces_the_ceiling() {
        let latency = LatencyCriterion::meeting()
            .at_most(Percentile::P95, Duration::from_millis(500))
            .at_most(Percentile::P95, Duration::from_millis(800));

        assert_eq!(
            latency.thresholds().get(Percentile::P95),
            Some(Duration::from_millis(800))
        );
    }

    #[test]
    #[should_panic(expected = "must be non-zero")]
    fn zero_ceiling_panics_at_the_setter() {
        let _ = LatencyCriterion::meeting().at_most(Percentile::P95, Duration::ZERO);
    }
}
