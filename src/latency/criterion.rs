//! The single latency criterion a contract may declare.

use std::time::Duration;

use crate::latency::percentile::Percentile;
use crate::latency::thresholds::LatencyThresholds;

/// A contract's latency commitment.
///
/// A contract declares **at most one** latency criterion — a service has a
/// single latency profile, so a single criterion holds the whole commitment.
/// That one criterion may still bound several percentiles
/// (`p95`, `p99`, …), each with its own ceiling, or assert them against the
/// baseline the test consumes.
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
///
/// or from [`empirical`](Self::empirical), declaring the percentiles whose
/// thresholds are derived from the baseline with
/// [`against_baseline`](Self::against_baseline):
///
/// ```
/// use feotest::latency::{LatencyCriterion, Percentile};
///
/// let latency = LatencyCriterion::empirical().against_baseline(Percentile::P95);
///
/// assert!(latency.is_against_baseline(Percentile::P95));
/// assert!(!latency.is_against_baseline(Percentile::P99));
/// ```
///
/// Every declared percentile is an assertion, enforced unless the run makes
/// the latency dimension advisory; a percentile declared neither way is not
/// asserted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyCriterion {
    thresholds: LatencyThresholds,
    against_baseline: [bool; Percentile::ALL.len()],
    confidence: f64,
}

impl LatencyCriterion {
    /// Begins a latency criterion whose ceilings are normative targets the
    /// service is required to meet. Each declared percentile is decided by
    /// `latency/compliance-exact-binomial`: the count
    /// of successful latencies at or below the ceiling must demonstrate, at
    /// the criterion's confidence, that at least the percentile's share of
    /// latencies meets it.
    ///
    /// Chain [`at_most`](Self::at_most) to bound one or more percentiles.
    #[must_use]
    pub const fn meeting() -> Self {
        Self {
            thresholds: LatencyThresholds::new(),
            against_baseline: [false; Percentile::ALL.len()],
            confidence: crate::latency::DEFAULT_LATENCY_CONFIDENCE,
        }
    }

    /// Begins a latency criterion whose thresholds are derived from the
    /// baseline the test consumes, each decided by `latency/precedence`
    /// after the run on the test's own successful latencies (the baseline
    /// latency an undegraded service exceeds with probability at most
    /// alpha).
    ///
    /// Chain [`against_baseline`](Self::against_baseline) to assert one or
    /// more percentiles.
    #[must_use]
    pub const fn empirical() -> Self {
        Self::meeting()
    }

    /// Asserts a percentile against the baseline: its threshold is derived
    /// from the baseline's successful latencies by `latency/precedence`. A
    /// percentile that also has an explicit ceiling ([`at_most`](Self::at_most))
    /// is that requirement instead.
    #[must_use]
    pub const fn against_baseline(mut self, percentile: Percentile) -> Self {
        self.against_baseline[percentile.index()] = true;
        self
    }

    /// Whether the percentile is asserted against the baseline.
    #[must_use]
    pub const fn is_against_baseline(&self, percentile: Percentile) -> bool {
        self.against_baseline[percentile.index()]
    }

    /// Sets the confidence level (`1 − alpha`) at which every ceiling of this
    /// criterion is decided. Defaults to 0.95.
    ///
    /// # Panics
    ///
    /// Panics if `confidence` is not in the open interval `(0, 1)`.
    #[must_use]
    pub fn confidence(mut self, confidence: f64) -> Self {
        assert!(
            confidence > 0.0 && confidence < 1.0,
            "latency confidence must be in (0, 1), got {confidence}"
        );
        self.confidence = confidence;
        self
    }

    /// The confidence level the ceilings are decided at.
    #[must_use]
    pub const fn decision_confidence(&self) -> f64 {
        self.confidence
    }

    /// Bounds a percentile at `max`: at least the percentile's share of the
    /// successful latencies must be at or below it.
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
    fn confidence_defaults_to_095_and_can_be_declared() {
        assert!((LatencyCriterion::meeting().decision_confidence() - 0.95).abs() < f64::EPSILON);
        let declared = LatencyCriterion::meeting().confidence(0.99);
        assert!((declared.decision_confidence() - 0.99).abs() < f64::EPSILON);
    }

    #[test]
    #[should_panic(expected = "latency confidence must be in (0, 1)")]
    fn rejects_a_confidence_outside_the_unit_interval() {
        let _ = LatencyCriterion::meeting().confidence(1.0);
    }

    #[test]
    fn empirical_starts_with_no_percentile_asserted() {
        let latency = LatencyCriterion::empirical();
        assert!(latency.thresholds().is_empty());
        assert!(
            Percentile::ALL
                .iter()
                .all(|&p| !latency.is_against_baseline(p))
        );
    }

    #[test]
    fn against_baseline_asserts_each_declared_percentile() {
        let latency = LatencyCriterion::empirical()
            .against_baseline(Percentile::P95)
            .against_baseline(Percentile::P50);
        let asserted: Vec<Percentile> = Percentile::ALL
            .into_iter()
            .filter(|&p| latency.is_against_baseline(p))
            .collect();
        assert_eq!(asserted, [Percentile::P50, Percentile::P95]);
    }

    #[test]
    #[should_panic(expected = "must be non-zero")]
    fn zero_ceiling_panics_at_the_setter() {
        let _ = LatencyCriterion::meeting().at_most(Percentile::P95, Duration::ZERO);
    }
}
