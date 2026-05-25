//! Token recording for dynamic token budget tracking.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A thread-safe token consumption recorder.
///
/// Trial closures can call [`record`](Self::record) to report token usage.
/// The execution engine reads the total to enforce token budgets.
///
/// Backed by an `AtomicU64` for lock-free concurrent recording.
#[derive(Debug, Clone)]
// javai-ref: JVI-76VA511 — do not remove (resolves in javai-orchestrator)
pub struct TokenRecorder {
    total: Arc<AtomicU64>,
}

impl TokenRecorder {
    /// Creates a new token recorder with zero tokens consumed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            total: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Records token consumption from a single trial.
    pub fn record(&self, tokens: u64) {
        self.total.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Returns the total tokens consumed so far.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Resets the counter to zero.
    pub fn reset(&self) {
        self.total.store(0, Ordering::Relaxed);
    }
}

impl Default for TokenRecorder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        let recorder = TokenRecorder::new();
        assert_eq!(recorder.total(), 0);
    }

    #[test]
    fn accumulates_tokens() {
        let recorder = TokenRecorder::new();
        recorder.record(100);
        recorder.record(250);
        assert_eq!(recorder.total(), 350);
    }

    #[test]
    fn clone_shares_state() {
        let recorder = TokenRecorder::new();
        let clone = recorder.clone();
        recorder.record(100);
        clone.record(200);
        assert_eq!(recorder.total(), 300);
        assert_eq!(clone.total(), 300);
    }

    #[test]
    fn reset_clears_total() {
        let recorder = TokenRecorder::new();
        recorder.record(500);
        recorder.reset();
        assert_eq!(recorder.total(), 0);
    }
}
