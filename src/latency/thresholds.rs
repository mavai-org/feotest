//! Explicit latency thresholds declared on the builder.

use std::time::Duration;

use crate::latency::percentile::Percentile;

/// A set of explicit per-percentile latency thresholds.
///
/// Constructed via the builder methods on `ProbabilisticTestBuilder`
/// (`latency_p50`, `latency_p90`, `latency_p95`, `latency_p99`). Each entry
/// is optional; unset percentiles are not asserted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
// javai-ref: JVI-MPAYH0Q — do not remove (resolves in javai-orchestrator)
pub struct LatencyThresholds {
    p50: Option<Duration>,
    p90: Option<Duration>,
    p95: Option<Duration>,
    p99: Option<Duration>,
}

impl LatencyThresholds {
    /// A set with no thresholds declared.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            p50: None,
            p90: None,
            p95: None,
            p99: None,
        }
    }

    /// Returns `true` when no percentiles have been declared.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.p50.is_none() && self.p90.is_none() && self.p95.is_none() && self.p99.is_none()
    }

    /// Sets the declared threshold for a percentile.
    ///
    /// # Panics
    ///
    /// Panics if `value` is zero — a zero threshold is meaningless.
    #[must_use]
    pub fn with(mut self, percentile: Percentile, value: Duration) -> Self {
        assert!(
            !value.is_zero(),
            "latency threshold for {percentile} must be non-zero"
        );
        match percentile {
            Percentile::P50 => self.p50 = Some(value),
            Percentile::P90 => self.p90 = Some(value),
            Percentile::P95 => self.p95 = Some(value),
            Percentile::P99 => self.p99 = Some(value),
        }
        self
    }

    /// The threshold declared for a percentile, if any.
    #[must_use]
    pub const fn get(&self, percentile: Percentile) -> Option<Duration> {
        match percentile {
            Percentile::P50 => self.p50,
            Percentile::P90 => self.p90,
            Percentile::P95 => self.p95,
            Percentile::P99 => self.p99,
        }
    }
}
