//! Verdict sinks: pluggable destinations for sentinel verdict records.
//!
//! A sink receives every [`VerdictRecord`] the sentinel runtime produces
//! and delivers it somewhere — a console line, a JSON-Lines file, an HTTP
//! webhook, or any combination of the three. The sentinel runner holds a
//! [`CompositeVerdictSink`] composed of whatever sinks the caller wires up
//! and fans every verdict out to each one.
//!
//! # Lifecycle
//!
//! Every sink passes through three phases, in order:
//!
//! 1. [`initialize`](VerdictSink::initialize) — opens files, validates
//!    endpoints, performs any one-time setup. Default is a no-op.
//! 2. [`accept`](VerdictSink::accept) — receives one verdict. Called once
//!    per verdict the runtime produces, in the order the runtime produces
//!    them.
//! 3. [`finalize`](VerdictSink::finalize) — flushes buffers, closes
//!    resources. Default is a no-op.
//!
//! # Failure isolation
//!
//! Sinks are advisory: a sink that fails to deliver does **not** abort the
//! sentinel run. When a sink returns [`SinkError`] from any lifecycle
//! method, the [`CompositeVerdictSink`] logs the error to stderr and
//! continues to the next sink. Programmatic users who call a single sink
//! directly can handle the error themselves; inside the runner, the
//! composite swallows it.
//!
//! # Built-in sinks
//!
//! - [`ConsoleVerdictSink`] — tab-separated verdict lines on stdout. The
//!   default when no sink is explicitly configured. Preserves the CLI
//!   output shape SN02 established.
//! - [`FileVerdictSink`] — one JSON-Lines record per verdict, appended to
//!   the configured path. Opens the file on [`initialize`] and closes it
//!   on [`finalize`]. No rotation.
//! - [`WebhookVerdictSink`] — POSTs each verdict as JSON. Requires the
//!   `webhook` Cargo feature to be enabled; otherwise not compiled in.
//! - [`CompositeVerdictSink`] — fans every lifecycle call out to a list of
//!   wrapped sinks. Errors are isolated.
//!
//! # Wire shape
//!
//! [`VerdictRecord`] serialises as a `camelCase` JSON object. The file and
//! webhook sinks use this shape directly. A minimal pass verdict looks
//! like:
//!
//! ```json
//! {
//!   "identity": { "useCaseId": "sla_demo.always_ok" },
//!   "verdict": "PASS",
//!   "verdictReason": "1.0000 >= 0.9500",
//!   "intent": "VERIFICATION",
//!   "execution": { ... },
//!   "functionalAssessment": {
//!     "composite": "PASS",
//!     "criteria": [ { "name": "result", "pass": 100, "fail": 0, "passRate": 1.0, "verdict": "PASS" } ]
//!   },
//!   "covariateStatus": { "aligned": true }
//! }
//! ```
//!
//! Fields that are empty or `None` are omitted to keep the payload small.

use crate::verdict::VerdictRecord;

pub mod composite;
pub mod console;
pub mod file;
#[cfg(feature = "webhook")]
pub mod webhook;

pub use composite::CompositeVerdictSink;
pub use console::ConsoleVerdictSink;
pub use file::FileVerdictSink;
#[cfg(feature = "webhook")]
pub use webhook::{WebhookVerdictSink, WebhookVerdictSinkBuilder};

/// Errors a [`VerdictSink`] may report back to the runner.
///
/// Sinks are expected to capture their own delivery failures and translate
/// them into one of these variants. The runner logs the error and
/// continues — no variant aborts the sentinel run.
#[derive(Debug, Clone)]
pub enum SinkError {
    /// The sink could not complete [`VerdictSink::initialize`] (e.g. the
    /// file path was unwritable, the webhook URL was malformed).
    InitialisationFailed(String),
    /// The sink could not deliver a verdict (e.g. HTTP non-2xx response,
    /// transport error, IO failure).
    DeliveryFailed(String),
    /// The sink could not complete [`VerdictSink::finalize`] (e.g. flush
    /// or close failed).
    FinalisationFailed(String),
}

impl core::fmt::Display for SinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InitialisationFailed(detail) => write!(f, "initialisation failed: {detail}"),
            Self::DeliveryFailed(detail) => write!(f, "delivery failed: {detail}"),
            Self::FinalisationFailed(detail) => write!(f, "finalisation failed: {detail}"),
        }
    }
}

impl std::error::Error for SinkError {}

/// A destination for verdict records produced by the sentinel runtime.
///
/// Implementations must be `Send + Sync` so the runner can hold a
/// trait-object list and pass each verdict to every sink without
/// additional synchronisation.
pub trait VerdictSink: Send + Sync {
    /// One-time setup before any verdicts are dispatched.
    ///
    /// Default is a no-op; sinks that need to open resources (file, HTTP
    /// client) override it.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError::InitialisationFailed`] when setup cannot
    /// complete. The runner logs and continues without this sink.
    fn initialize(&mut self) -> Result<(), SinkError> {
        Ok(())
    }

    /// Accepts one verdict record.
    ///
    /// Implementations should not propagate lower-level failures as
    /// panics — translate them into [`SinkError::DeliveryFailed`] so the
    /// runner can isolate the failure.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError::DeliveryFailed`] when the verdict cannot be
    /// delivered to the underlying destination.
    fn accept(&mut self, verdict: &VerdictRecord) -> Result<(), SinkError>;

    /// One-time cleanup after the last verdict has been accepted.
    ///
    /// Default is a no-op; sinks that buffer or hold resources override
    /// it.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError::FinalisationFailed`] when cleanup cannot
    /// complete. The runner logs the failure; the run's exit status is
    /// unaffected.
    fn finalize(&mut self) -> Result<(), SinkError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_error_display_includes_detail() {
        let err = SinkError::DeliveryFailed("connection refused".to_string());
        assert_eq!(err.to_string(), "delivery failed: connection refused");

        let err = SinkError::InitialisationFailed("bad path".to_string());
        assert_eq!(err.to_string(), "initialisation failed: bad path");

        let err = SinkError::FinalisationFailed("flush failed".to_string());
        assert_eq!(err.to_string(), "finalisation failed: flush failed");
    }
}
