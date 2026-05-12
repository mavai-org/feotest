//! Console verdict sink: emits one tab-separated line per verdict.

use std::io::{self, Stdout, Write};

use crate::sentinel::sinks::{SinkError, VerdictSink};
use crate::verdict::VerdictRecord;

/// Writes each verdict as a single tab-separated line.
///
/// This sink's output is the sentinel CLI's default verdict shape:
/// `{useCaseId}\t{testName|-}\t{verdict}`. The shape is documented so
/// existing scripts that parse the CLI output keep working.
///
/// The writer is generic so tests can substitute a `Vec<u8>` buffer for
/// stdout. [`ConsoleVerdictSink::new`] yields the common case — a sink
/// that writes to stdout.
pub struct ConsoleVerdictSink<W: Write + Send + Sync = Stdout> {
    writer: W,
}

impl ConsoleVerdictSink<Stdout> {
    /// Creates a console sink that writes to the process stdout.
    #[must_use]
    pub fn new() -> Self {
        Self {
            writer: io::stdout(),
        }
    }
}

impl Default for ConsoleVerdictSink<Stdout> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Write + Send + Sync> ConsoleVerdictSink<W> {
    /// Creates a console sink that writes to an arbitrary writer.
    ///
    /// Intended for tests and for callers that want to redirect verdict
    /// output (e.g. to a log buffer). The writer is moved in; use
    /// [`into_writer`](Self::into_writer) to reclaim it.
    pub const fn to_writer(writer: W) -> Self {
        Self { writer }
    }

    /// Consumes the sink and returns the underlying writer.
    pub fn into_writer(self) -> W {
        self.writer
    }
}

impl<W: Write + Send + Sync> VerdictSink for ConsoleVerdictSink<W> {
    fn accept(&mut self, verdict: &VerdictRecord) -> Result<(), SinkError> {
        writeln!(
            self.writer,
            "{}\t{}\t{:?}",
            verdict.identity().service_contract_id(),
            verdict.identity().test_name().unwrap_or("-"),
            verdict.verdict()
        )
        .map_err(|e| SinkError::DeliveryFailed(e.to_string()))
    }

    fn finalize(&mut self) -> Result<(), SinkError> {
        self.writer
            .flush()
            .map_err(|e| SinkError::FinalisationFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity, TestIntent,
    };
    use crate::verdict::{FunctionalDimension, Verdict, VerdictRecord};
    use std::time::Duration;

    fn sample_execution() -> ExecutionSummary {
        ExecutionSummary::new(
            100,
            100,
            95,
            5,
            TerminationInfo::new(TerminationReason::Completed),
            CostSummary::new(Duration::from_millis(500), 1000, 100),
        )
    }

    fn pass_verdict() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("basket").with_test_name("translates"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .build()
    }

    #[test]
    fn accept_writes_one_tab_separated_line() {
        let mut sink = ConsoleVerdictSink::to_writer(Vec::<u8>::new());
        sink.accept(&pass_verdict()).expect("accept");
        let buf = String::from_utf8(sink.into_writer()).expect("utf8");
        assert_eq!(buf, "basket\ttranslates\tPass\n");
    }

    #[test]
    fn accept_uses_dash_when_test_name_is_missing() {
        let record = VerdictRecord::builder(
            TestIdentity::new("basket"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(0, 100, vec![]),
        )
        .build();

        let mut sink = ConsoleVerdictSink::to_writer(Vec::<u8>::new());
        sink.accept(&record).expect("accept");
        let buf = String::from_utf8(sink.into_writer()).expect("utf8");
        assert_eq!(buf, "basket\t-\tFail\n");
    }

    #[test]
    fn finalize_flushes_writer() {
        let mut sink = ConsoleVerdictSink::to_writer(Vec::<u8>::new());
        sink.accept(&pass_verdict()).expect("accept");
        sink.finalize().expect("finalize");
    }
}
