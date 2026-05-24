//! File verdict sink: appends one JSON-Lines record per verdict.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::sentinel::sinks::{SinkError, VerdictSink};
use crate::verdict::VerdictRecord;

/// Appends every verdict to a file as one JSON record per line.
///
/// The target path is captured at construction and opened lazily on
/// [`initialize`](VerdictSink::initialize). Subsequent verdicts are
/// written append-only, one JSON object per line. On
/// [`finalize`](VerdictSink::finalize) the buffered writer is flushed
/// and dropped.
///
/// No rotation, size limit, or back-pressure is applied — callers that
/// need durability should consider composing the file sink with a
/// webhook sink in parallel.
pub struct FileVerdictSink {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
}

impl FileVerdictSink {
    /// Creates a new file sink targeting the given path.
    ///
    /// The path is not opened until [`initialize`](VerdictSink::initialize)
    /// is called, so construction is infallible.
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            path: path.into(),
            writer: None,
        }
    }

    /// The path this sink writes to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl VerdictSink for FileVerdictSink {
    fn initialize(&mut self) -> Result<(), SinkError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| {
                SinkError::InitialisationFailed(format!(
                    "failed to open {}: {e}",
                    self.path.display()
                ))
            })?;
        self.writer = Some(BufWriter::new(file));
        Ok(())
    }

    fn accept(&mut self, verdict: &VerdictRecord) -> Result<(), SinkError> {
        let writer = self.writer.as_mut().ok_or_else(|| {
            SinkError::DeliveryFailed("file sink was not initialised".to_string())
        })?;
        let line = serde_json::to_string(verdict)
            .map_err(|e| SinkError::DeliveryFailed(format!("json encode failed: {e}")))?;
        writeln!(writer, "{line}").map_err(|e| SinkError::DeliveryFailed(e.to_string()))
    }

    fn finalize(&mut self) -> Result<(), SinkError> {
        if let Some(mut writer) = self.writer.take() {
            writer
                .flush()
                .map_err(|e| SinkError::FinalisationFailed(e.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity, TestIntent,
    };
    use crate::verdict::{CriterionRow, FunctionalAssessment, Verdict, VerdictRecord};
    use std::io::{BufRead, BufReader};
    use std::time::Duration;

    fn sample_record(service_contract: &str, verdict: Verdict) -> VerdictRecord {
        let exec = ExecutionSummary::new(
            100,
            100,
            95,
            5,
            TerminationInfo::new(TerminationReason::Completed),
            CostSummary::new(Duration::from_millis(200), 500, 100),
        );
        VerdictRecord::builder(
            TestIdentity::new(service_contract),
            verdict,
            TestIntent::Verification,
            exec,
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], verdict)),
        )
        .build()
    }

    #[test]
    fn writes_one_json_line_per_verdict_and_reopens_as_append() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("verdicts.jsonl");

        let mut sink = FileVerdictSink::new(&path);
        sink.initialize().expect("initialize");
        sink.accept(&sample_record("one", Verdict::Pass))
            .expect("first accept");
        sink.accept(&sample_record("two", Verdict::Fail))
            .expect("second accept");
        sink.finalize().expect("finalize");

        let file = File::open(&path).expect("reopen");
        let lines: Vec<String> = BufReader::new(file)
            .lines()
            .collect::<Result<_, _>>()
            .expect("read lines");
        assert_eq!(lines.len(), 2, "expected two JSON lines: {lines:?}");

        let first: serde_json::Value = serde_json::from_str(&lines[0]).expect("first line is json");
        assert_eq!(first["identity"]["useCaseId"], "one");
        assert_eq!(first["verdict"], "PASS");

        let second: serde_json::Value =
            serde_json::from_str(&lines[1]).expect("second line is json");
        assert_eq!(second["identity"]["useCaseId"], "two");
        assert_eq!(second["verdict"], "FAIL");

        // A second sink should append rather than truncate.
        let mut second_sink = FileVerdictSink::new(&path);
        second_sink.initialize().expect("second init");
        second_sink
            .accept(&sample_record("three", Verdict::Inconclusive))
            .expect("append");
        second_sink.finalize().expect("second finalize");

        let line_count = BufReader::new(File::open(&path).expect("reopen2"))
            .lines()
            .count();
        assert_eq!(line_count, 3, "expected append, not truncate");
    }

    #[test]
    fn accept_without_initialize_returns_delivery_failed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("uninit.jsonl");

        let mut sink = FileVerdictSink::new(&path);
        let err = sink
            .accept(&sample_record("x", Verdict::Pass))
            .expect_err("uninitialised sink should error");
        assert!(matches!(err, SinkError::DeliveryFailed(_)));
    }

    #[test]
    fn initialize_fails_with_a_helpful_message_on_bad_path() {
        // A path containing a NUL byte cannot be opened on any platform.
        let mut sink = FileVerdictSink::new("/\0/definitely/not/a/path.jsonl");
        let err = sink.initialize().expect_err("expected init failure");
        assert!(
            matches!(err, SinkError::InitialisationFailed(_)),
            "expected InitialisationFailed, got {err}"
        );
    }
}
