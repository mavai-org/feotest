//! Run-scoped budget: time and token caps applied across every test
//! executed in a single `cargo test` invocation.
//!
//! A run-scoped budget is opt-out. When a budget is configured — either
//! from the process environment or via an explicit setter — every test
//! that reaches the execution engine is automatically subject to it.
//! When no budget is configured, the run is uncapped at this scope.
//!
//! The companion per-method budgets on [`ExecutionConfig`](crate::controls::ExecutionConfig)
//! coexist with the run-scoped budget via a first-exhausted-wins rule:
//! the execution engine checks each configured budget before every
//! sample, and the first to be exhausted stops the test.

use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Environment variable for the run-scoped time budget, in milliseconds.
pub const TIME_BUDGET_ENV: &str = "FEOTEST_RUN_TIME_BUDGET_MS";

/// Environment variable for the run-scoped token budget.
pub const TOKEN_BUDGET_ENV: &str = "FEOTEST_RUN_TOKEN_BUDGET";

/// A budget shared by every test executed in one `cargo test` invocation.
///
/// Wraps the two cap parameters (time, tokens), a fixed wall-clock start
/// instant, and an atomic token counter. The predicates
/// [`time_exhausted`](Self::time_exhausted) and
/// [`token_exhausted_at`](Self::token_exhausted_at) are what the
/// execution engine consults inside its sample loop.
///
/// `RunBudget` is thread-safe; consumption recording uses
/// `AtomicU64::fetch_add` with `Relaxed` ordering, matching the
/// per-method [`TokenRecorder`](crate::controls::TokenRecorder).
#[derive(Debug)]
pub struct RunBudget {
    time_budget: Option<Duration>,
    token_budget: Option<u64>,
    start: Instant,
    tokens_consumed: AtomicU64,
}

impl RunBudget {
    /// Creates a run budget with the given caps.
    ///
    /// Either cap may be `None` (unconstrained at this scope) but a
    /// `Some` value must be strictly positive. The wall-clock start
    /// instant is stamped to `Instant::now()` at the point of
    /// construction.
    ///
    /// # Panics
    ///
    /// Panics if `time_budget` is `Some(Duration::ZERO)` or if
    /// `token_budget` is `Some(0)`.
    #[must_use]
    pub fn new(time_budget: Option<Duration>, token_budget: Option<u64>) -> Self {
        if let Some(t) = time_budget {
            assert!(
                !t.is_zero(),
                "run-scoped time budget must be positive, got {t:?}"
            );
        }
        if let Some(n) = token_budget {
            assert!(n > 0, "run-scoped token budget must be positive, got 0");
        }
        Self {
            time_budget,
            token_budget,
            start: Instant::now(),
            tokens_consumed: AtomicU64::new(0),
        }
    }

    /// Reads a run budget from the process environment.
    ///
    /// Returns `None` when neither [`TIME_BUDGET_ENV`] nor
    /// [`TOKEN_BUDGET_ENV`] is set. When either is set, its value must
    /// parse as a positive integer; any other value aborts the process
    /// with a descriptive panic, treating a misconfigured cost gate as
    /// a defect in whichever harness set the variable.
    ///
    /// # Panics
    ///
    /// Panics if a set variable is unparseable as `u64` or parses to
    /// zero.
    #[must_use]
    pub fn from_environment() -> Option<Self> {
        let time_ms = env::var(TIME_BUDGET_ENV)
            .ok()
            .map(|raw| parse_positive_u64(&raw, TIME_BUDGET_ENV));
        let tokens = env::var(TOKEN_BUDGET_ENV)
            .ok()
            .map(|raw| parse_positive_u64(&raw, TOKEN_BUDGET_ENV));
        if time_ms.is_none() && tokens.is_none() {
            None
        } else {
            Some(Self::new(time_ms.map(Duration::from_millis), tokens))
        }
    }

    /// The configured time cap, if any.
    #[must_use]
    pub const fn time_budget(&self) -> Option<Duration> {
        self.time_budget
    }

    /// The configured token cap, if any.
    #[must_use]
    pub const fn token_budget(&self) -> Option<u64> {
        self.token_budget
    }

    /// Wall-clock time elapsed since this budget was constructed.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Cumulative tokens recorded against this budget.
    #[must_use]
    pub fn tokens_consumed(&self) -> u64 {
        self.tokens_consumed.load(Ordering::Relaxed)
    }

    /// Adds to the cumulative token consumption.
    ///
    /// Thread-safe; uses `Relaxed` ordering. `record_tokens(0)` is a
    /// no-op.
    pub fn record_tokens(&self, n: u64) {
        if n > 0 {
            self.tokens_consumed.fetch_add(n, Ordering::Relaxed);
        }
    }

    /// Whether the time budget is set and already exhausted.
    ///
    /// Returns `false` when no time budget is configured.
    #[must_use]
    pub fn time_exhausted(&self) -> bool {
        self.time_budget.is_some_and(|b| self.elapsed() >= b)
    }

    /// Whether the token budget would be exhausted after
    /// `projected_additional` more tokens are added.
    ///
    /// Returns `false` when no token budget is configured. Uses
    /// saturating addition so an overflowing projection is treated as
    /// exhaustion rather than silently wrapping.
    #[must_use]
    pub fn token_exhausted_at(&self, projected_additional: u64) -> bool {
        self.token_budget.is_some_and(|b| {
            self.tokens_consumed()
                .saturating_add(projected_additional)
                >= b
        })
    }
}

/// Parses a positive `u64` from the raw string value of an environment
/// variable. Panics with a message that names the variable when the
/// value is unparseable or zero.
fn parse_positive_u64(raw: &str, name: &str) -> u64 {
    let n: u64 = raw
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a positive integer, got {raw:?}"));
    assert!(n > 0, "{name} must be a positive integer, got 0");
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    // NOTE on env-var coverage: the workspace forbids `unsafe` code, and
    // `std::env::set_var` / `remove_var` became `unsafe` in Rust 1.82 due
    // to their inherent races with concurrent readers. Unit-level tests
    // therefore exercise the pure parser and the constructor directly;
    // end-to-end env-var ingestion is covered by the subprocess-based
    // integration test in `tests/`.

    #[test]
    fn parse_positive_u64_accepts_valid_input() {
        assert_eq!(parse_positive_u64("1500", "ANY"), 1_500);
        assert_eq!(parse_positive_u64("1", "ANY"), 1);
    }

    #[test]
    #[should_panic(expected = "FEOTEST_RUN_TIME_BUDGET_MS must be a positive integer, got \"abc\"")]
    fn parse_positive_u64_rejects_non_numeric() {
        let _ = parse_positive_u64("abc", TIME_BUDGET_ENV);
    }

    #[test]
    #[should_panic(expected = "FEOTEST_RUN_TOKEN_BUDGET must be a positive integer, got 0")]
    fn parse_positive_u64_rejects_zero() {
        let _ = parse_positive_u64("0", TOKEN_BUDGET_ENV);
    }

    #[test]
    #[should_panic(expected = "must be a positive integer, got \"-5\"")]
    fn parse_positive_u64_rejects_negative() {
        let _ = parse_positive_u64("-5", TIME_BUDGET_ENV);
    }

    #[test]
    #[should_panic(expected = "run-scoped time budget must be positive")]
    fn new_rejects_zero_time_budget() {
        let _ = RunBudget::new(Some(Duration::ZERO), None);
    }

    #[test]
    #[should_panic(expected = "run-scoped token budget must be positive")]
    fn new_rejects_zero_token_budget() {
        let _ = RunBudget::new(None, Some(0));
    }

    #[test]
    fn accessors_reflect_constructor_args() {
        let budget = RunBudget::new(Some(Duration::from_secs(30)), Some(50_000));
        assert_eq!(budget.time_budget(), Some(Duration::from_secs(30)));
        assert_eq!(budget.token_budget(), Some(50_000));
        assert_eq!(budget.tokens_consumed(), 0);
    }

    #[test]
    fn time_exhausted_false_when_no_budget() {
        let budget = RunBudget::new(None, None);
        assert!(!budget.time_exhausted());
    }

    #[test]
    fn time_exhausted_true_once_elapsed() {
        let budget = RunBudget::new(Some(Duration::from_millis(1)), None);
        thread::sleep(Duration::from_millis(10));
        assert!(budget.time_exhausted());
    }

    #[test]
    fn token_exhausted_false_when_no_budget() {
        let budget = RunBudget::new(None, None);
        assert!(!budget.token_exhausted_at(u64::MAX));
    }

    #[test]
    fn token_exhausted_at_composes_projection() {
        let budget = RunBudget::new(None, Some(100));
        budget.record_tokens(60);
        assert!(!budget.token_exhausted_at(39));
        assert!(budget.token_exhausted_at(40));
    }

    #[test]
    fn token_exhausted_at_saturates_on_overflow() {
        let budget = RunBudget::new(None, Some(100));
        budget.record_tokens(50);
        assert!(budget.token_exhausted_at(u64::MAX));
    }

    #[test]
    fn record_tokens_accumulates() {
        let budget = RunBudget::new(None, Some(1_000));
        budget.record_tokens(100);
        budget.record_tokens(250);
        assert_eq!(budget.tokens_consumed(), 350);
    }

    #[test]
    fn record_tokens_zero_is_noop() {
        let budget = RunBudget::new(None, Some(1_000));
        budget.record_tokens(0);
        assert_eq!(budget.tokens_consumed(), 0);
    }
}
