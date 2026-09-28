//! Resolves explicit and baseline-derived latency constraints into a single
//! list of constraints to judge after the run.
//!
//! A baseline-derived threshold depends on the number of successful
//! latencies the test itself collects (`latency/precedence`, Statistical
//! Companion §12.4), so it is derived after the run, not here: the resolver
//! carries the baseline's latencies forward and the latency dimension
//! derives and judges.

use std::time::Duration;

use serde::Serialize;

use crate::latency::enforcement::LatencyEnforcementMode;
use crate::latency::percentile::Percentile;
use crate::latency::thresholds::LatencyThresholds;
use crate::spec::baseline::LatencyBlock;

/// Where a judged threshold came from, as the verdict records it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThresholdProvenance {
    /// The contract declared the threshold explicitly.
    Explicit,
    /// Derived from the baseline's successful latencies by
    /// `latency/precedence`.
    BaselineDerived {
        /// Confidence level of the precedence rank (`1 − alpha`).
        confidence: f64,
        /// 1-indexed baseline rank the threshold sits at; `None` when no
        /// rank achieves alpha (saturated).
        #[serde(skip_serializing_if = "Option::is_none")]
        rank: Option<u32>,
        /// Number of baseline successful latencies.
        n: u32,
    },
}

/// Where a constraint's threshold comes from, before the run.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintSource {
    /// A threshold the contract declares.
    Explicit {
        /// The declared ceiling (inclusive).
        threshold: Duration,
    },
    /// A threshold to be derived from the baseline's successful latencies.
    BaselineDerived {
        /// The baseline's successful latencies in milliseconds, ascending.
        baseline_latencies_ms: Vec<f64>,
    },
}

/// A latency constraint ready to be judged on the run's successful
/// latencies.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLatencyConstraint {
    percentile: Percentile,
    source: ConstraintSource,
    mode: LatencyEnforcementMode,
    confidence: f64,
}

impl ResolvedLatencyConstraint {
    /// The percentile this constraint targets.
    #[must_use]
    pub const fn percentile(&self) -> Percentile {
        self.percentile
    }

    /// Where the threshold comes from.
    #[must_use]
    pub const fn source(&self) -> &ConstraintSource {
        &self.source
    }

    /// The enforcement mode that governs this constraint.
    #[must_use]
    pub const fn mode(&self) -> LatencyEnforcementMode {
        self.mode
    }

    /// The confidence level of the constraint's decision (`1 − alpha`).
    #[must_use]
    pub const fn confidence(&self) -> f64 {
        self.confidence
    }

    /// Whether the constraint consumes the baseline's latencies.
    #[must_use]
    pub const fn is_baseline_derived(&self) -> bool {
        matches!(self.source, ConstraintSource::BaselineDerived { .. })
    }

    /// Whether the constraint takes part in the verdict.
    #[must_use]
    pub fn is_enforced(&self) -> bool {
        self.mode == LatencyEnforcementMode::Strict
    }
}

/// The confidence levels the resolved constraints are decided at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConstraintConfidence {
    /// For the contract's explicit latency requirements.
    pub explicit: f64,
    /// For thresholds derived from the baseline.
    pub baseline: f64,
}

/// Resolves explicit and baseline-derived constraints into one list.
///
/// - An explicit threshold wins over the baseline for its percentile and is
///   enforced strictly (`latency/compliance-exact-binomial`).
/// - A percentile the contract does not bound gets a baseline-derived
///   constraint when the baseline recorded successful latencies, governed by
///   `mode_for_baseline` (`latency/precedence`).
#[must_use]
// mavai-ref: JVI-QVNG2SX — do not remove (resolves in mavai-orchestrator)
pub fn resolve(
    explicit: &LatencyThresholds,
    baseline: Option<&LatencyBlock>,
    confidence: ConstraintConfidence,
    mode_for_baseline: LatencyEnforcementMode,
) -> Vec<ResolvedLatencyConstraint> {
    let baseline_latencies: Option<Vec<f64>> = baseline
        .filter(|block| !block.latencies_ms.is_empty())
        .map(|block| {
            #[allow(
                clippy::cast_precision_loss,
                reason = "millisecond latencies fit in f64 mantissa"
            )]
            let latencies = block.latencies_ms.iter().map(|&ms| ms as f64).collect();
            latencies
        });
    Percentile::ALL
        .iter()
        .filter_map(|&percentile| {
            if let Some(threshold) = explicit.get(percentile) {
                return Some(ResolvedLatencyConstraint {
                    percentile,
                    source: ConstraintSource::Explicit { threshold },
                    mode: LatencyEnforcementMode::Strict,
                    confidence: confidence.explicit,
                });
            }
            baseline_latencies
                .as_ref()
                .map(|latencies| ResolvedLatencyConstraint {
                    percentile,
                    source: ConstraintSource::BaselineDerived {
                        baseline_latencies_ms: latencies.clone(),
                    },
                    mode: mode_for_baseline,
                    confidence: confidence.baseline,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(latencies: Vec<u64>) -> LatencyBlock {
        LatencyBlock {
            mean_ms: 0,
            max_ms: latencies.last().copied().unwrap_or(0),
            latencies_ms: latencies,
        }
    }

    const CONFIDENCE: ConstraintConfidence = ConstraintConfidence {
        explicit: 0.95,
        baseline: 0.99,
    };

    #[test]
    fn explicit_thresholds_win_and_are_enforced() {
        let explicit = LatencyThresholds::new().with(Percentile::P95, Duration::from_millis(500));
        let baseline = block((1..=100).collect());
        let resolved = resolve(
            &explicit,
            Some(&baseline),
            CONFIDENCE,
            LatencyEnforcementMode::Advisory,
        );
        assert_eq!(resolved.len(), 4);
        let p95 = resolved
            .iter()
            .find(|c| c.percentile() == Percentile::P95)
            .unwrap();
        assert!(p95.is_enforced());
        assert!(!p95.is_baseline_derived());
        assert!((p95.confidence() - 0.95).abs() < f64::EPSILON);
        let p99 = resolved
            .iter()
            .find(|c| c.percentile() == Percentile::P99)
            .unwrap();
        assert!(p99.is_baseline_derived());
        assert!(!p99.is_enforced());
        assert!((p99.confidence() - 0.99).abs() < f64::EPSILON);
    }

    #[test]
    fn a_baseline_without_latencies_derives_nothing() {
        let resolved = resolve(
            &LatencyThresholds::new(),
            Some(&block(Vec::new())),
            CONFIDENCE,
            LatencyEnforcementMode::Strict,
        );
        assert!(resolved.is_empty());
    }

    #[test]
    fn no_baseline_and_no_explicit_thresholds_resolve_to_nothing() {
        let resolved = resolve(
            &LatencyThresholds::new(),
            None,
            CONFIDENCE,
            LatencyEnforcementMode::Strict,
        );
        assert!(resolved.is_empty());
    }
}
