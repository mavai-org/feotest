//! Percentile enumeration for the latency dimension.

use std::fmt;

/// The set of percentiles that may be asserted on a latency dimension.
///
/// Matches the levels declared in the reference oracle
/// (`javai-R/R/latency.R`) and in punit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Percentile {
    /// 50th percentile (median).
    P50,
    /// 90th percentile.
    P90,
    /// 95th percentile.
    P95,
    /// 99th percentile.
    P99,
}

impl Percentile {
    /// All supported percentiles in ascending order.
    pub const ALL: [Self; 4] = [Self::P50, Self::P90, Self::P95, Self::P99];

    /// The percentile expressed as a fraction in (0, 1].
    #[must_use]
    pub const fn as_fraction(self) -> f64 {
        match self {
            Self::P50 => 0.50,
            Self::P90 => 0.90,
            Self::P95 => 0.95,
            Self::P99 => 0.99,
        }
    }

    /// Short label (e.g. `p95`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::P50 => "p50",
            Self::P90 => "p90",
            Self::P95 => "p95",
            Self::P99 => "p99",
        }
    }
}

impl fmt::Display for Percentile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}
