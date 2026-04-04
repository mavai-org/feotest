//! Shared utilities for spec output across experiment types.

use std::time::SystemTime;

use crate::model::{CostSummary, ExecutionSummary, SampleAggregate};
use crate::spec::baseline::{CostBlock, ExecutionBlock};
use crate::statistics::types::ConfidenceLevel;
use crate::statistics::{defaults, proportion};

/// Round to 4 decimal places for spec output.
#[must_use]
pub fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

/// Simple ISO 8601 timestamp (no chrono dependency).
#[must_use]
pub fn now_iso8601() -> String {
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let (year, month, day) = days_to_date(secs / 86400);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Converts days since Unix epoch to (year, month, day).
/// Civil date algorithm from Howard Hinnant.
const fn days_to_date(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Builds a cost block from an execution summary.
#[must_use]
pub fn build_cost_block(cost: &CostSummary) -> CostBlock {
    CostBlock {
        total_time_ms: u64::try_from(cost.total_time().as_millis()).unwrap_or(u64::MAX),
        avg_time_per_sample_ms: u64::try_from(cost.avg_time_per_sample().as_millis())
            .unwrap_or(u64::MAX),
        total_tokens: cost.total_tokens(),
        avg_tokens_per_sample: cost.avg_tokens_per_sample(),
    }
}

/// Builds an execution block from an execution summary and planned sample count.
#[must_use]
pub fn build_execution_block(summary: &ExecutionSummary, samples_planned: u32) -> ExecutionBlock {
    ExecutionBlock {
        samples_planned,
        samples_executed: summary.samples_executed(),
        termination_reason: Some(summary.termination().reason().to_string()),
    }
}

/// Builds a sorted failure distribution map from a sample aggregate.
#[must_use]
pub fn build_failure_distribution(
    aggregate: &SampleAggregate,
) -> Option<std::collections::BTreeMap<String, u32>> {
    if aggregate.failure_distribution().is_empty() {
        None
    } else {
        let mut map = std::collections::BTreeMap::new();
        for (check, count) in aggregate.failure_distribution() {
            map.insert(check.clone(), *count);
        }
        Some(map)
    }
}

/// Computes the Wilson lower bound at 95% confidence.
#[must_use]
pub fn wilson_lower_bound(successes: u32, total: u32) -> f64 {
    let confidence = ConfidenceLevel::new(defaults::DEFAULT_CONFIDENCE);
    proportion::lower_bound(successes, total, confidence)
}

/// Computes the Wilson score confidence interval at 95% confidence.
#[must_use]
pub fn wilson_interval(successes: u32, total: u32) -> (f64, f64) {
    let confidence = ConfidenceLevel::new(defaults::DEFAULT_CONFIDENCE);
    let estimate = proportion::estimate(successes, total, confidence);
    (estimate.lower_bound(), estimate.upper_bound())
}

/// Computes the standard error of the observed proportion.
#[must_use]
pub fn standard_error(successes: u32, total: u32) -> f64 {
    proportion::standard_error(successes, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round4_works() {
        assert!((round4(0.123_456_789) - 0.1235).abs() < 1e-10);
        assert!((round4(0.5) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn now_iso8601_produces_valid_format() {
        let ts = now_iso8601();
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
        assert_eq!(ts.len(), 20);
    }
}
