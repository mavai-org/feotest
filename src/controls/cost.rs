//! The cost handle threaded through a service invocation.

use crate::controls::TokenRecorder;

/// A handle for reporting the token cost a single service invocation incurs.
///
/// A `Cost` is threaded into a service call so the contract can report the
/// tokens that call consumed via [`record_tokens`](Self::record_tokens). The
/// execution engine reads the accumulated total to enforce token budgets.
///
/// "Token" here is the framework's generic unit of cost — LLM tokens are the
/// obvious case, but a token may proxy any per-call charge.
///
/// ```
/// use feotest::controls::Cost;
///
/// let mut cost = Cost::new();
/// cost.record_tokens(150);
/// cost.record_tokens(50);
/// assert_eq!(cost.tokens_recorded(), 200);
/// ```
#[derive(Debug, Clone, Default)]
// mavai-ref: JVI-W6A4WRA — do not remove (resolves in mavai-orchestrator)
pub struct Cost {
    recorder: TokenRecorder,
}

impl Cost {
    /// Creates a cost handle backed by a fresh recorder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            recorder: TokenRecorder::new(),
        }
    }

    /// Records the tokens a single service invocation consumed.
    ///
    /// Charges accumulate; call once per invocation with that call's cost.
    pub fn record_tokens(&mut self, tokens: u64) {
        self.recorder.record(tokens);
    }

    /// The total tokens recorded so far through this handle.
    #[must_use]
    pub fn tokens_recorded(&self) -> u64 {
        self.recorder.total()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        assert_eq!(Cost::new().tokens_recorded(), 0);
    }

    #[test]
    fn accumulates_recorded_tokens() {
        let mut cost = Cost::new();
        cost.record_tokens(120);
        cost.record_tokens(30);
        assert_eq!(cost.tokens_recorded(), 150);
    }
}
