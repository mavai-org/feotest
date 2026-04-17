//! Resolves explicit and baseline-derived latency thresholds into a single
//! list of evaluation targets.

use std::time::Duration;

use crate::latency::enforcement::LatencyEnforcementMode;
use crate::latency::percentile::Percentile;
use crate::latency::thresholds::LatencyThresholds;
use crate::spec::baseline::LatencyBlock;
use crate::statistics::latency;

/// Where a resolved threshold came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThresholdProvenance {
    /// User declared the threshold explicitly on the builder.
    Explicit,
    /// Derived from the baseline spec at the given confidence level, landing
    /// on rank `k` of the baseline sample.
    BaselineDerived {
        /// Confidence level used for the binomial bound.
        confidence: f64,
        /// 1-indexed order-statistic rank chosen.
        rank: u32,
        /// Size of the baseline sample.
        n: u32,
    },
}

/// A percentile threshold that has been resolved to a concrete duration and
/// is ready for evaluation against observed latencies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedLatencyThreshold {
    percentile: Percentile,
    threshold: Duration,
    provenance: ThresholdProvenance,
    mode: LatencyEnforcementMode,
    feasible: bool,
}

impl ResolvedLatencyThreshold {
    /// The percentile this threshold targets.
    #[must_use]
    pub const fn percentile(&self) -> Percentile {
        self.percentile
    }

    /// The threshold value (inclusive upper bound).
    #[must_use]
    pub const fn threshold(&self) -> Duration {
        self.threshold
    }

    /// Where this threshold came from.
    #[must_use]
    pub const fn provenance(&self) -> ThresholdProvenance {
        self.provenance
    }

    /// The enforcement mode that governs this threshold.
    #[must_use]
    pub const fn mode(&self) -> LatencyEnforcementMode {
        self.mode
    }

    /// Whether the baseline held enough successful samples for the percentile
    /// estimate to be non-degenerate. Always `true` for `Explicit` thresholds.
    #[must_use]
    pub const fn feasible(&self) -> bool {
        self.feasible
    }
}

/// Resolves explicit and baseline-derived thresholds into one list.
///
/// - Explicit thresholds always win over baseline-derived ones for the same
///   percentile and are marked `Strict`, `feasible = true`.
/// - Baseline-derived thresholds call the non-parametric binomial bound in
///   `statistics::latency::derive_latency_threshold` and inherit
///   `mode_for_baseline`.
/// - Percentiles that lack enough baseline samples (see `min_samples_for`)
///   are still returned but flagged `feasible = false`; no threshold is
///   emitted for them (the caller reports this as a warning).
#[must_use]
pub fn resolve(
    explicit: &LatencyThresholds,
    baseline: Option<&LatencyBlock>,
    baseline_confidence: f64,
    mode_for_baseline: LatencyEnforcementMode,
) -> Vec<ResolvedLatencyThreshold> {
    let mut out = Vec::new();

    for &p in &Percentile::ALL {
        if let Some(value) = explicit.get(p) {
            out.push(ResolvedLatencyThreshold {
                percentile: p,
                threshold: value,
                provenance: ThresholdProvenance::Explicit,
                mode: LatencyEnforcementMode::Strict,
                feasible: true,
            });
            continue;
        }

        if let Some(block) = baseline {
            let n = u32::try_from(block.latencies_ms.len()).unwrap_or(u32::MAX);
            let min = latency::min_samples_for(p.as_fraction());
            if n < min {
                // Infeasible: record with a sentinel threshold; caller emits warning.
                out.push(ResolvedLatencyThreshold {
                    percentile: p,
                    threshold: Duration::ZERO,
                    provenance: ThresholdProvenance::BaselineDerived {
                        confidence: baseline_confidence,
                        rank: 0,
                        n,
                    },
                    mode: mode_for_baseline,
                    feasible: false,
                });
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let latencies_f64: Vec<f64> = block.latencies_ms.iter().map(|&x| x as f64).collect();
            let derived = latency::derive_latency_threshold(
                &latencies_f64,
                p.as_fraction(),
                baseline_confidence,
            );
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let threshold_ms = derived.threshold() as u64;
            out.push(ResolvedLatencyThreshold {
                percentile: p,
                threshold: Duration::from_millis(threshold_ms),
                provenance: ThresholdProvenance::BaselineDerived {
                    confidence: baseline_confidence,
                    rank: derived.rank(),
                    n: derived.n(),
                },
                mode: mode_for_baseline,
                feasible: true,
            });
        }
    }

    out
}
