//! Latency dimension of a verdict record.

use std::fmt;
use std::time::Duration;

use crate::latency::enforcement::LatencyEnforcementMode;
use crate::latency::percentile::Percentile;
use crate::latency::resolver::{ResolvedLatencyThreshold, ThresholdProvenance};
use crate::statistics::latency;

/// Per-evaluation status within the latency dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationStatus {
    /// Observed percentile is within the threshold.
    Pass,
    /// Observed percentile exceeds a strictly-enforced threshold.
    StrictFail,
    /// Observed percentile exceeds an advisory threshold.
    AdvisoryWarn,
    /// Baseline did not provide enough successful samples for the percentile
    /// estimate; no evaluation was performed.
    Infeasible,
}

/// A single evaluation of an observed percentile against its threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyEvaluation {
    percentile: Percentile,
    observed: Option<Duration>,
    threshold: Duration,
    provenance: ThresholdProvenance,
    mode: LatencyEnforcementMode,
    status: EvaluationStatus,
}

impl LatencyEvaluation {
    /// The percentile evaluated.
    #[must_use]
    pub const fn percentile(&self) -> Percentile {
        self.percentile
    }

    /// The observed percentile value, if computed.
    #[must_use]
    pub const fn observed(&self) -> Option<Duration> {
        self.observed
    }

    /// The threshold this observation was compared to.
    #[must_use]
    pub const fn threshold(&self) -> Duration {
        self.threshold
    }

    /// Where the threshold came from.
    #[must_use]
    pub const fn provenance(&self) -> ThresholdProvenance {
        self.provenance
    }

    /// Enforcement mode for this evaluation.
    #[must_use]
    pub const fn mode(&self) -> LatencyEnforcementMode {
        self.mode
    }

    /// The evaluation outcome.
    #[must_use]
    pub const fn status(&self) -> EvaluationStatus {
        self.status
    }
}

/// The latency dimension of a verdict record.
#[derive(Debug, Clone)]
pub struct LatencyDimension {
    observed_percentiles: Vec<(Percentile, Duration)>,
    evaluations: Vec<LatencyEvaluation>,
    strict_violations: u32,
    advisory_violations: u32,
    successful_samples: u32,
}

impl LatencyDimension {
    /// Builds a latency dimension from observed successful-response latencies
    /// and a list of resolved thresholds.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn build(successful_latencies_ms: &[f64], resolved: &[ResolvedLatencyThreshold]) -> Self {
        let successful_samples = successful_latencies_ms.len() as u32;

        // Pre-compute observed percentiles for every percentile mentioned.
        let mut observed_percentiles: Vec<(Percentile, Duration)> = Vec::new();
        for t in resolved {
            if observed_percentiles
                .iter()
                .any(|(p, _)| *p == t.percentile())
            {
                continue;
            }
            if successful_samples == 0 {
                continue;
            }
            let v = latency::nearest_rank_percentile(
                successful_latencies_ms,
                t.percentile().as_fraction(),
            );
            observed_percentiles.push((t.percentile(), Duration::from_millis(v.round() as u64)));
        }

        let mut evaluations = Vec::with_capacity(resolved.len());
        let mut strict_violations = 0u32;
        let mut advisory_violations = 0u32;

        for t in resolved {
            if !t.feasible() {
                evaluations.push(LatencyEvaluation {
                    percentile: t.percentile(),
                    observed: None,
                    threshold: t.threshold(),
                    provenance: t.provenance(),
                    mode: t.mode(),
                    status: EvaluationStatus::Infeasible,
                });
                continue;
            }
            let observed = observed_percentiles
                .iter()
                .find(|(p, _)| *p == t.percentile())
                .map(|(_, v)| *v);
            let status = match observed {
                None => EvaluationStatus::Infeasible,
                Some(v) if v <= t.threshold() => EvaluationStatus::Pass,
                Some(_) => match t.mode() {
                    LatencyEnforcementMode::Strict => {
                        strict_violations += 1;
                        EvaluationStatus::StrictFail
                    }
                    LatencyEnforcementMode::Advisory => {
                        advisory_violations += 1;
                        EvaluationStatus::AdvisoryWarn
                    }
                },
            };
            evaluations.push(LatencyEvaluation {
                percentile: t.percentile(),
                observed,
                threshold: t.threshold(),
                provenance: t.provenance(),
                mode: t.mode(),
                status,
            });
        }

        Self {
            observed_percentiles,
            evaluations,
            strict_violations,
            advisory_violations,
            successful_samples,
        }
    }

    /// Whether the latency dimension passed (no strict violations).
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.strict_violations == 0
    }

    /// Number of strict threshold violations.
    #[must_use]
    pub const fn strict_violations(&self) -> u32 {
        self.strict_violations
    }

    /// Number of advisory threshold violations.
    #[must_use]
    pub const fn advisory_violations(&self) -> u32 {
        self.advisory_violations
    }

    /// Per-percentile observed values, one entry per percentile present in
    /// the evaluation set.
    #[must_use]
    pub fn observed_percentiles(&self) -> &[(Percentile, Duration)] {
        &self.observed_percentiles
    }

    /// The evaluations performed in resolution order.
    #[must_use]
    pub fn evaluations(&self) -> &[LatencyEvaluation] {
        &self.evaluations
    }

    /// Number of post-warmup successful samples used for percentile
    /// computation.
    #[must_use]
    pub const fn successful_samples(&self) -> u32 {
        self.successful_samples
    }
}

impl fmt::Display for LatencyDimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Latency dimension ({} samples):",
            self.successful_samples
        )?;
        for ev in &self.evaluations {
            let obs = ev
                .observed
                .map_or_else(|| "—".to_string(), |d| format!("{} ms", d.as_millis()));
            let thr = format!("{} ms", ev.threshold.as_millis());
            let status = match ev.status {
                EvaluationStatus::Pass => "PASS",
                EvaluationStatus::StrictFail => "FAIL",
                EvaluationStatus::AdvisoryWarn => "WARN",
                EvaluationStatus::Infeasible => "INFEASIBLE",
            };
            let provenance = match ev.provenance {
                ThresholdProvenance::Explicit => "explicit".to_string(),
                ThresholdProvenance::BaselineDerived {
                    confidence,
                    rank,
                    n,
                } => {
                    format!("baseline rank={rank}/{n} c={confidence:.2}")
                }
            };
            writeln!(
                f,
                "  {}: observed={obs}, threshold={thr} [{provenance}] -> {status}",
                ev.percentile
            )?;
        }
        if self.advisory_violations > 0 {
            writeln!(
                f,
                "  advisory violations: {} (do not affect verdict)",
                self.advisory_violations
            )?;
        }
        Ok(())
    }
}
