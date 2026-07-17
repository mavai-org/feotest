//! Shared utilities for spec output across experiment types.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::model::{CostSummary, ExecutionSummary, SampleAggregate};
use crate::spec::baseline::{CostBlock, ExecutionBlock, LatencyBlock, SuccessRateBlock};
use crate::spec::keys;
use crate::statistics::types::ConfidenceLevel;
use crate::statistics::{defaults, proportion};

/// Seconds in a day (no leap seconds — ISO 8601 timestamps here treat
/// the day as a fixed 86400-second interval).
const SECONDS_PER_DAY: u64 = 86_400;

/// Round to 4 decimal places for spec output.
#[must_use]
pub fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

/// Simple ISO 8601 timestamp (no chrono dependency).
#[must_use]
pub fn now_iso8601() -> String {
    format_iso8601(SystemTime::now())
}

/// Formats a [`SystemTime`] as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Times before the Unix epoch are clamped to `1970-01-01T00:00:00Z`.
#[must_use]
pub fn format_iso8601(time: SystemTime) -> String {
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    let time_secs = secs % SECONDS_PER_DAY;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let (year, month, day) = days_to_date(secs / SECONDS_PER_DAY);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Parses an ISO 8601 timestamp in the `YYYY-MM-DDTHH:MM:SSZ` form
/// produced by [`format_iso8601`].
///
/// Returns `None` for any other format. Intentionally strict — this is
/// not a general-purpose ISO 8601 parser, only the inverse of the
/// writer.
#[must_use]
pub fn parse_iso8601(s: &str) -> Option<SystemTime> {
    let bytes = s.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }

    let year = parse_digits(&bytes[0..4])?;
    let month = parse_digits(&bytes[5..7])?;
    let day = parse_digits(&bytes[8..10])?;
    let hour = parse_digits(&bytes[11..13])?;
    let minute = parse_digits(&bytes[14..16])?;
    let second = parse_digits(&bytes[17..19])?;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour >= 24
        || minute >= 60
        || second >= 60
    {
        return None;
    }

    let days = date_to_days(year, month, day);
    let secs = days * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

fn parse_digits(bytes: &[u8]) -> Option<u64> {
    let mut n: u64 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + u64::from(b - b'0');
    }
    Some(n)
}

/// Returns `baseline_end_time + days` formatted as ISO 8601, or `None`
/// if the input cannot be parsed.
#[must_use]
pub fn iso8601_plus_days(baseline_end_time: &str, days: u32) -> Option<String> {
    let start = parse_iso8601(baseline_end_time)?;
    let later = start + Duration::from_secs(u64::from(days) * SECONDS_PER_DAY);
    Some(format_iso8601(later))
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

/// Inverse of [`days_to_date`]: returns days since Unix epoch.
///
/// Valid for dates on or after `1970-01-01`.
const fn date_to_days(year: u64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let m = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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

/// One entry of a `failureDistribution` sequence in interchange output.
///
/// The interchange formats carry failure attribution as a *sequence* of
/// entries rather than a mapping, so no mapping key can grow with runtime
/// content. `condition` is the violating condition's bounded identity —
/// never embedding input or response content; per-input attribution, where
/// an emitter tracks it, travels structurally in `input_index` with an
/// optional bounded excerpt beside it in `input_excerpt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureDistributionEntry {
    /// The violating condition's bounded identity.
    pub condition: String,

    /// Zero-based index into the run's inputs list, when the entry is
    /// attributed to a specific input. The input value itself is never
    /// serialised as identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_index: Option<u32>,

    /// Informational bounded excerpt of the driving input, for human
    /// orientation only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_excerpt: Option<String>,

    /// Failed trials attributed to this entry.
    pub count: u32,
}

/// Builds the failure distribution sequence from a sample aggregate.
///
/// Each failed trial was attributed to its first failing condition when the
/// aggregate recorded it, so the entries' counts sum to the aggregate's
/// failure total. Condition identities are bounded per the artefact key
/// discipline. Returns `None` when no failures were recorded.
#[must_use]
pub fn build_failure_distribution(
    aggregate: &SampleAggregate,
) -> Option<Vec<FailureDistributionEntry>> {
    build_failure_entries(aggregate.failure_distribution().iter().cloned())
}

/// Builds a failure distribution sequence from `(condition, count)` tallies.
///
/// Each condition identity is bounded; tallies whose identities collapse to
/// the same bounded form are merged. Returns `None` for an empty tally.
#[must_use]
pub fn build_failure_entries(
    tallies: impl IntoIterator<Item = (String, u32)>,
) -> Option<Vec<FailureDistributionEntry>> {
    let mut entries: Vec<FailureDistributionEntry> = Vec::new();
    for (condition, count) in tallies {
        let condition = keys::bounded_identity(&condition);
        if let Some(entry) = entries.iter_mut().find(|e| e.condition == condition) {
            entry.count += count;
        } else {
            entries.push(FailureDistributionEntry {
                condition,
                input_index: None,
                input_excerpt: None,
                count,
            });
        }
    }
    (!entries.is_empty()).then_some(entries)
}

/// Builds a sorted failure distribution map from a sample aggregate, with
/// mapping keys bounded per the artefact key discipline.
///
/// The baseline spec still carries its failure distribution as a mapping;
/// bounding the keys keeps the emitted document valid YAML even when a
/// violating check's name grew with runtime content.
#[must_use]
pub fn build_failure_distribution_map(
    aggregate: &SampleAggregate,
) -> Option<std::collections::BTreeMap<String, u32>> {
    bounded_distribution_map(aggregate.failure_distribution().iter().cloned())
}

/// Collects `(check, count)` tallies into a map with bounded keys, merging
/// tallies whose keys collapse to the same bounded form. Returns `None`
/// when there are no tallies.
#[must_use]
pub fn bounded_distribution_map(
    tallies: impl IntoIterator<Item = (String, u32)>,
) -> Option<std::collections::BTreeMap<String, u32>> {
    let mut map = std::collections::BTreeMap::new();
    for (check, count) in tallies {
        *map.entry(keys::bounded_identity(&check)).or_insert(0) += count;
    }
    (!map.is_empty()).then_some(map)
}

/// Builds a success rate block from raw success and total counts.
#[must_use]
pub fn build_success_rate_block(successes: u32, total: u32) -> SuccessRateBlock {
    let observed = if total == 0 {
        0.0
    } else {
        f64::from(successes) / f64::from(total)
    };
    let se = standard_error(successes, total);
    let (ci_lower, ci_upper) = wilson_interval(successes, total);
    SuccessRateBlock {
        observed: round4(observed),
        standard_error: round4(se),
        confidence_interval95: [round4(ci_lower), round4(ci_upper)],
    }
}

/// Builds a latency distribution block from successful-response latencies.
///
/// Returns `None` when no successes were recorded.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::missing_panics_doc,
    reason = "ms latencies fit in f64; empty branch returns before .expect"
)]
pub fn build_latency_distribution(
    successful_latencies: &[std::time::Duration],
) -> Option<LatencyBlock> {
    if successful_latencies.is_empty() {
        return None;
    }
    let mut ms: Vec<u64> = successful_latencies
        .iter()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .collect();
    ms.sort_unstable();
    let n = ms.len() as f64;
    let sum: f64 = ms.iter().map(|&x| x as f64).sum();
    let mean_ms = (sum / n).round() as u64;
    let max_ms = *ms.last().expect("non-empty");
    Some(LatencyBlock {
        latencies_ms: ms,
        mean_ms,
        max_ms,
    })
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

    #[test]
    fn iso8601_round_trips_known_date() {
        // 2026-04-19 is 20_562 days after 1970-01-01.
        let ts = "2026-04-19T10:30:45Z";
        let parsed = parse_iso8601(ts).unwrap();
        let formatted = format_iso8601(parsed);
        assert_eq!(formatted, ts);
    }

    #[test]
    fn iso8601_round_trips_now() {
        let ts = now_iso8601();
        let parsed = parse_iso8601(&ts).unwrap();
        let formatted = format_iso8601(parsed);
        assert_eq!(formatted, ts);
    }

    #[test]
    fn parse_iso8601_rejects_wrong_length() {
        assert!(parse_iso8601("2026-04-19").is_none());
        assert!(parse_iso8601("2026-04-19T10:30:45").is_none());
        assert!(parse_iso8601("2026-04-19T10:30:45ZZ").is_none());
    }

    #[test]
    fn parse_iso8601_rejects_bad_separators() {
        assert!(parse_iso8601("2026/04/19T10:30:45Z").is_none());
        assert!(parse_iso8601("2026-04-19X10:30:45Z").is_none());
    }

    #[test]
    fn parse_iso8601_rejects_out_of_range() {
        assert!(parse_iso8601("2026-13-19T10:30:45Z").is_none());
        assert!(parse_iso8601("2026-04-19T25:30:45Z").is_none());
    }

    #[test]
    fn iso8601_plus_days_crosses_month_boundary() {
        // 2026-04-19 + 15 days = 2026-05-04
        let result = iso8601_plus_days("2026-04-19T10:00:00Z", 15).unwrap();
        assert_eq!(result, "2026-05-04T10:00:00Z");
    }

    #[test]
    fn iso8601_plus_days_handles_zero() {
        let result = iso8601_plus_days("2026-04-19T10:00:00Z", 0).unwrap();
        assert_eq!(result, "2026-04-19T10:00:00Z");
    }

    #[test]
    fn iso8601_plus_days_crosses_year_boundary() {
        // 2025-12-28 + 10 days = 2026-01-07
        let result = iso8601_plus_days("2025-12-28T08:00:00Z", 10).unwrap();
        assert_eq!(result, "2026-01-07T08:00:00Z");
    }

    #[test]
    fn failure_entries_preserve_order_and_counts() {
        let entries = build_failure_entries([("parse".to_owned(), 2), ("empty".to_owned(), 1)])
            .expect("non-empty tally");
        assert_eq!(entries.len(), 2);
        assert_eq!(
            (entries[0].condition.as_str(), entries[0].count),
            ("parse", 2)
        );
        assert_eq!(
            (entries[1].condition.as_str(), entries[1].count),
            ("empty", 1)
        );
        assert!(entries[0].input_index.is_none());
    }

    #[test]
    fn failure_entries_are_none_for_an_empty_tally() {
        assert!(build_failure_entries(std::iter::empty()).is_none());
    }

    #[test]
    fn failure_entries_bound_over_long_conditions() {
        let long = "c".repeat(2_000);
        let entries = build_failure_entries([(long, 3)]).expect("non-empty tally");
        assert_eq!(entries[0].condition.chars().count(), keys::MAX_KEY_CHARS);
        assert_eq!(entries[0].count, 3);
    }

    #[test]
    fn failure_entries_merge_tallies_with_the_same_bounded_condition() {
        let long = "c".repeat(2_000);
        let entries =
            build_failure_entries([(long.clone(), 2), (long, 1)]).expect("non-empty tally");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].count, 3);
    }

    #[test]
    fn bounded_distribution_map_bounds_its_keys() {
        let long = "k".repeat(2_000);
        let map =
            bounded_distribution_map([(long, 1), ("short".to_owned(), 2)]).expect("non-empty");
        assert_eq!(map.len(), 2);
        assert!(map.keys().all(|k| k.chars().count() <= keys::MAX_KEY_CHARS));
        assert_eq!(map.get("short"), Some(&2));
    }

    #[test]
    fn bounded_distribution_map_is_none_when_empty() {
        assert!(bounded_distribution_map(std::iter::empty()).is_none());
    }
}
