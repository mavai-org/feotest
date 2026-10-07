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

use crate::latency::criterion::LatencyCriterion;
use crate::latency::percentile::Percentile;
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
        /// The baseline's successful latencies in milliseconds, ascending;
        /// empty when no baseline with latencies resolved, which leaves the
        /// constraint undecidable (INCONCLUSIVE).
        baseline_latencies_ms: Vec<f64>,
    },
}

/// A latency constraint ready to be judged on the run's successful
/// latencies.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedLatencyConstraint {
    percentile: Percentile,
    source: ConstraintSource,
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

    /// Whether the constraint is asserted against a baseline that recorded
    /// no successful latencies (or against no baseline at all).
    #[must_use]
    pub fn lacks_baseline_latencies(&self) -> bool {
        matches!(
            &self.source,
            ConstraintSource::BaselineDerived { baseline_latencies_ms } if baseline_latencies_ms.is_empty()
        )
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

/// Resolves the declared latency criterion into the constraints judged after
/// the run.
///
/// - An explicit ceiling is a requirement (`latency/compliance-exact-binomial`)
///   and wins over the baseline for its percentile.
/// - A percentile asserted against the baseline gets a baseline-derived
///   constraint (`latency/precedence`) carrying the baseline's successful
///   latencies — none when no baseline with latencies resolved.
/// - A percentile declared neither way is not asserted.
///
/// Every constraint is a declared assertion; whether the latency dimension
/// binds the test is the run's choice, not the constraint's.
#[must_use]
// mavai-ref: JVI-QVNG2SX — do not remove (resolves in mavai-orchestrator)
pub fn resolve(
    declared: &LatencyCriterion,
    baseline: Option<&LatencyBlock>,
    confidence: ConstraintConfidence,
) -> Vec<ResolvedLatencyConstraint> {
    #[allow(
        clippy::cast_precision_loss,
        reason = "millisecond latencies fit in f64 mantissa"
    )]
    let baseline_latencies: Vec<f64> = baseline
        .map(|block| block.latencies_ms.iter().map(|&ms| ms as f64).collect())
        .unwrap_or_default();
    Percentile::ALL
        .iter()
        .filter_map(|&percentile| {
            if let Some(threshold) = declared.thresholds().get(percentile) {
                return Some(ResolvedLatencyConstraint {
                    percentile,
                    source: ConstraintSource::Explicit { threshold },
                    confidence: confidence.explicit,
                });
            }
            declared
                .is_against_baseline(percentile)
                .then(|| ResolvedLatencyConstraint {
                    percentile,
                    source: ConstraintSource::BaselineDerived {
                        baseline_latencies_ms: baseline_latencies.clone(),
                    },
                    confidence: confidence.baseline,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
    fn explicit_thresholds_win_over_the_baseline() {
        let declared = LatencyCriterion::meeting()
            .at_most(Percentile::P95, Duration::from_millis(500))
            .against_baseline(Percentile::P95)
            .against_baseline(Percentile::P99);
        let baseline = block((1..=100).collect());
        let resolved = resolve(&declared, Some(&baseline), CONFIDENCE);
        assert_eq!(resolved.len(), 2);
        let p95 = &resolved[0];
        assert_eq!(p95.percentile(), Percentile::P95);
        assert!(!p95.is_baseline_derived());
        assert!((p95.confidence() - 0.95).abs() < f64::EPSILON);
        let p99 = &resolved[1];
        assert_eq!(p99.percentile(), Percentile::P99);
        assert!(p99.is_baseline_derived());
        assert!(!p99.lacks_baseline_latencies());
        assert!((p99.confidence() - 0.99).abs() < f64::EPSILON);
    }

    #[test]
    fn only_declared_percentiles_are_asserted() {
        let baseline = block((1..=100).collect());
        let resolved = resolve(&LatencyCriterion::empirical(), Some(&baseline), CONFIDENCE);
        assert_eq!(resolved.len(), 0);
    }

    #[test]
    fn a_baseline_without_latencies_leaves_a_declared_constraint_without_them() {
        let declared = LatencyCriterion::empirical().against_baseline(Percentile::P50);
        let from_empty = resolve(&declared, Some(&block(Vec::new())), CONFIDENCE);
        assert!(from_empty[0].lacks_baseline_latencies());
        let from_none = resolve(&declared, None, CONFIDENCE);
        assert!(from_none[0].lacks_baseline_latencies());
    }
}
