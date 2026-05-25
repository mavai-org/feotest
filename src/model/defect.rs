//! Defects: the abort channel for a service invocation.

use std::fmt;

/// A defect: no response could be obtained from the service at all.
///
/// A service invocation returns `Err(Defect)` only when *no* response is
/// obtainable — a transport failure, or a panic-class fault caught during the
/// call. A defect aborts the run, subject to the configured exception policy.
///
/// A malformed-but-received response is **not** a defect: it is returned as the
/// raw output and judged (and parsed) by a criterion, where a parse failure is
/// a counted contract failure rather than an abort. The deciding question is
/// whether a response came back at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defect {
    message: String,
}

impl Defect {
    /// Creates a defect with a human-readable explanation.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Why no response could be obtained.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Defect {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_its_message() {
        let defect = Defect::new("connection refused");
        assert_eq!(defect.message(), "connection refused");
        assert_eq!(defect.to_string(), "connection refused");
    }
}
