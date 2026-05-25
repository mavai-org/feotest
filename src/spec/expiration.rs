//! Evaluates baseline freshness against the current (or a supplied) time.
//!
//! A baseline spec carries an optional [`ExpirationBlock`] recording when
//! the measurement ran and how long it stays representative. At test time
//! the runner calls [`evaluate`] (or [`evaluate_at`] for tests that want
//! to pin the clock) and attaches the returned [`ExpirationInfo`] to the
//! verdict's [`crate::verdict::SpecProvenance`].
//!
//! The evaluator is deliberately isolated from reporting and runner
//! concerns: it takes a spec and a time, returns a status. How that
//! status is surfaced — warning, failure, report line — is the caller's
//! decision.

use std::time::{Duration, SystemTime};

use crate::model::{ExpirationInfo, ExpirationStatus};
use crate::spec::baseline::{BaselineSpec, ExpirationBlock};
use crate::spec::common::parse_iso8601;

/// Threshold below which a baseline is "expiring imminently" (≤ 10 %
/// of the validity window remaining). Follows the shared baseline scheme so
/// that cross-framework verdicts can be compared directly.
const IMMINENT_REMAINING_RATIO: f64 = 0.10;

/// Threshold below which a baseline is "expiring soon" (≤ 25 % of
/// the validity window remaining).
const SOON_REMAINING_RATIO: f64 = 0.25;

/// Evaluates the expiration status of the given baseline as of now.
///
/// Equivalent to `evaluate_at(spec, SystemTime::now())`.
#[must_use]
pub fn evaluate(spec: &BaselineSpec) -> ExpirationInfo {
    evaluate_at(spec, SystemTime::now())
}

/// Evaluates the expiration status of the given baseline at a caller-
/// supplied instant. Useful for deterministic tests of boundary
/// conditions.
#[must_use]
// javai-ref: JVI-09GQGN$ — do not remove (resolves in javai-orchestrator)
pub fn evaluate_at(spec: &BaselineSpec, now: SystemTime) -> ExpirationInfo {
    let Some(block) = spec.expiration.as_ref() else {
        return ExpirationInfo::new(ExpirationStatus::NoExpiration, None);
    };
    evaluate_block(block, now)
}

fn evaluate_block(block: &ExpirationBlock, now: SystemTime) -> ExpirationInfo {
    if block.expires_in_days == 0 {
        // Defensive: the writer never emits a zero-day block, but a hand-
        // edited YAML could. Treat as no expiration to match the semantics
        // of omitting the block altogether.
        return ExpirationInfo::new(ExpirationStatus::NoExpiration, None);
    }

    let Some(end_time) = parse_iso8601(&block.baseline_end_time) else {
        // Unparseable timestamp in the spec — treat as no expiration so
        // we don't fabricate a status. This is a programmer/authoring
        // error, surfaced by separate validation in the future.
        return ExpirationInfo::new(ExpirationStatus::NoExpiration, None);
    };

    let window = Duration::from_secs(u64::from(block.expires_in_days) * SECONDS_PER_DAY);
    let expiration_at = end_time + window;
    let expires_at = Some(block.expiration_date.clone());

    let status = if now >= expiration_at {
        ExpirationStatus::Expired
    } else {
        let remaining = expiration_at.duration_since(now).unwrap_or(Duration::ZERO);
        let remaining_ratio = duration_ratio(remaining, window);
        if remaining_ratio <= IMMINENT_REMAINING_RATIO {
            ExpirationStatus::ExpiringImminently
        } else if remaining_ratio <= SOON_REMAINING_RATIO {
            ExpirationStatus::ExpiringSoon
        } else {
            ExpirationStatus::Valid
        }
    };

    ExpirationInfo::new(status, expires_at)
}

const SECONDS_PER_DAY: u64 = 86_400;

#[allow(
    clippy::cast_precision_loss,
    reason = "millisecond durations fit in f64 mantissa"
)]
fn duration_ratio(part: Duration, whole: Duration) -> f64 {
    let whole_ms = whole.as_millis();
    if whole_ms == 0 {
        return 0.0;
    }
    part.as_millis() as f64 / whole_ms as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::baseline::{
        BaselineSpec, ExecutionBlock, RequirementsBlock, StatisticsBlock, SuccessRateBlock,
    };

    fn spec_with_expiration(expires_in_days: u32, baseline_end_time: &str) -> BaselineSpec {
        let mut spec = BaselineSpec::new(
            "uc",
            baseline_end_time,
            ExecutionBlock {
                samples_planned: 100,
                samples_executed: 100,
                termination_reason: None,
            },
            RequirementsBlock { min_pass_rate: 0.9 },
            StatisticsBlock {
                success_rate: SuccessRateBlock {
                    observed: 0.95,
                    standard_error: 0.01,
                    confidence_interval95: [0.9, 1.0],
                },
                successes: 95,
                failures: 5,
                failure_distribution: None,
                latency_distribution: None,
            },
        );
        if expires_in_days > 0 {
            let expiration_date =
                crate::spec::common::iso8601_plus_days(baseline_end_time, expires_in_days).unwrap();
            spec.expiration = Some(ExpirationBlock {
                expires_in_days,
                baseline_end_time: baseline_end_time.to_owned(),
                expiration_date,
            });
        }
        spec
    }

    #[test]
    fn no_block_yields_no_expiration() {
        let spec = spec_with_expiration(0, "2026-04-19T10:00:00Z");
        let info = evaluate(&spec);
        assert_eq!(info.status(), &ExpirationStatus::NoExpiration);
        assert!(info.expires_at().is_none());
    }

    #[test]
    fn just_past_expiry_is_expired() {
        let end = "2026-04-19T10:00:00Z";
        let spec = spec_with_expiration(1, end);
        // Expiry is 2026-04-20T10:00:00Z; evaluate one second after.
        let past = parse_iso8601("2026-04-20T10:00:01Z").unwrap();
        let info = evaluate_at(&spec, past);
        assert_eq!(info.status(), &ExpirationStatus::Expired);
        assert_eq!(info.expires_at(), Some("2026-04-20T10:00:00Z"));
    }

    #[test]
    fn exactly_at_expiry_is_expired() {
        let end = "2026-04-19T10:00:00Z";
        let spec = spec_with_expiration(1, end);
        let exactly = parse_iso8601("2026-04-20T10:00:00Z").unwrap();
        let info = evaluate_at(&spec, exactly);
        assert_eq!(info.status(), &ExpirationStatus::Expired);
    }

    #[test]
    fn just_before_expiry_is_imminent() {
        // 100-day window, 5 days before the end → 5 % remaining.
        let end = "2026-04-01T00:00:00Z";
        let spec = spec_with_expiration(100, end);
        let now = parse_iso8601("2026-07-05T00:00:00Z").unwrap();
        let info = evaluate_at(&spec, now);
        assert_eq!(info.status(), &ExpirationStatus::ExpiringImminently);
    }

    #[test]
    fn fifteen_percent_remaining_is_soon() {
        // 100-day window, 15 days before the end → 15 % remaining.
        let end = "2026-01-01T00:00:00Z";
        let spec = spec_with_expiration(100, end);
        let now = parse_iso8601("2026-03-27T00:00:00Z").unwrap();
        let info = evaluate_at(&spec, now);
        assert_eq!(info.status(), &ExpirationStatus::ExpiringSoon);
    }

    #[test]
    fn well_within_window_is_valid() {
        let end = "2026-04-01T00:00:00Z";
        let spec = spec_with_expiration(100, end);
        let now = parse_iso8601("2026-04-15T00:00:00Z").unwrap();
        let info = evaluate_at(&spec, now);
        assert_eq!(info.status(), &ExpirationStatus::Valid);
    }

    #[test]
    fn exact_soon_boundary_is_soon() {
        // 25 % remaining on a 100-day window = 25 days left.
        let end = "2026-04-01T00:00:00Z";
        let spec = spec_with_expiration(100, end);
        let now = parse_iso8601("2026-06-15T00:00:00Z").unwrap();
        let info = evaluate_at(&spec, now);
        assert_eq!(info.status(), &ExpirationStatus::ExpiringSoon);
    }

    #[test]
    fn exact_imminent_boundary_is_imminent() {
        // 10 % remaining on a 100-day window = 10 days left.
        let end = "2026-04-01T00:00:00Z";
        let spec = spec_with_expiration(100, end);
        let now = parse_iso8601("2026-06-30T00:00:00Z").unwrap();
        let info = evaluate_at(&spec, now);
        assert_eq!(info.status(), &ExpirationStatus::ExpiringImminently);
    }

    #[test]
    fn zero_day_block_treated_as_no_expiration() {
        // Hand-crafted spec where the block is present but asserts no
        // window: parity with omitting the block.
        let mut spec = spec_with_expiration(0, "2026-04-19T10:00:00Z");
        spec.expiration = Some(ExpirationBlock {
            expires_in_days: 0,
            baseline_end_time: "2026-04-19T10:00:00Z".to_owned(),
            expiration_date: "2026-04-19T10:00:00Z".to_owned(),
        });
        let info = evaluate(&spec);
        assert_eq!(info.status(), &ExpirationStatus::NoExpiration);
    }

    #[test]
    fn unparseable_baseline_end_time_is_no_expiration() {
        let mut spec = spec_with_expiration(0, "2026-04-19T10:00:00Z");
        spec.expiration = Some(ExpirationBlock {
            expires_in_days: 30,
            baseline_end_time: "not-a-timestamp".to_owned(),
            expiration_date: "not-a-timestamp".to_owned(),
        });
        let info = evaluate(&spec);
        assert_eq!(info.status(), &ExpirationStatus::NoExpiration);
    }
}
