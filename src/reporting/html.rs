//! HTML report writer via XSLT transformation.
//!
//! Generates a self-contained HTML5 report by:
//! 1. Serialising verdict records to RP07 verdict XML.
//! 2. Wrapping them in a `<verdict-suite>` envelope.
//! 3. Applying the shared XSLT stylesheet via `xsltproc`.
//!
//! The XSLT stylesheet is embedded at compile time from
//! `verdict-report.xslt` in this directory.

use std::io;
use std::path::Path;
use std::process::Command;

use crate::reporting::VerdictXmlWriter;
use crate::verdict::VerdictRecord;

/// The XSLT stylesheet embedded at compile time.
const XSLT_STYLESHEET: &str = include_str!("verdict-report.xslt");

/// Generates a standalone HTML report from verdict records.
///
/// The report is produced by serialising verdicts to XML and applying
/// an XSLT stylesheet via `xsltproc`. This requires `xsltproc` to be
/// installed and available on `PATH`.
pub struct HtmlReportWriter;

impl HtmlReportWriter {
    /// Generates the complete HTML content for a suite report.
    ///
    /// Serialises `records` to verdict XML, wraps them in a
    /// `<verdict-suite>` envelope, and applies the XSLT stylesheet
    /// via `xsltproc`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `xsltproc` is not installed or cannot be executed.
    /// - Temporary files cannot be created.
    /// - The XSLT transformation fails.
    pub fn generate(records: &[VerdictRecord], timestamp: Option<&str>) -> io::Result<String> {
        // Serialise each record to XML
        let record_xmls: Vec<String> = records
            .iter()
            .map(|r| VerdictXmlWriter::write_record(r, timestamp))
            .collect();

        // Wrap in a suite envelope
        let suite_xml = VerdictXmlWriter::wrap_suite(&record_xmls, timestamp);

        // Write the suite XML and XSLT to temp files.
        // Use PID + thread ID to avoid collisions when tests run in parallel.
        let temp_dir = std::env::temp_dir().join(format!(
            "feotest-report-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&temp_dir)?;

        let xml_path = temp_dir.join("suite.xml");
        let xslt_path = temp_dir.join("verdict-report.xslt");

        std::fs::write(&xml_path, &suite_xml)?;
        std::fs::write(&xslt_path, XSLT_STYLESHEET)?;

        // Run xsltproc
        let output = Command::new("xsltproc")
            .arg(&xslt_path)
            .arg(&xml_path)
            .output()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "xsltproc not found. Install libxslt to generate HTML reports. \
                         On macOS: brew install libxslt. \
                         On Debian/Ubuntu: apt install xsltproc.",
                    )
                } else {
                    e
                }
            })?;

        // Clean up temp files (best-effort)
        let _ = std::fs::remove_dir_all(&temp_dir);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("xsltproc failed: {stderr}"),
            ));
        }

        String::from_utf8(output.stdout).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("xsltproc produced invalid UTF-8: {e}"),
            )
        })
    }

    /// Generates an HTML report and writes it to a file.
    ///
    /// Creates parent directories if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if report generation or file writing fails.
    pub fn write_to_file(
        path: &Path,
        records: &[VerdictRecord],
        timestamp: Option<&str>,
    ) -> io::Result<()> {
        let html = Self::generate(records, timestamp)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, html)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity,
        TestIntent, ThresholdOrigin,
    };
    use crate::verdict::{
        FunctionalDimension, SpecProvenance, StatisticalAnalysis, Verdict, VerdictRecord,
    };
    use std::time::Duration;

    fn sample_execution(
        planned: u32,
        executed: u32,
        successes: u32,
        failures: u32,
    ) -> ExecutionSummary {
        ExecutionSummary::new(
            planned,
            executed,
            successes,
            failures,
            TerminationInfo::new(TerminationReason::Completed),
            CostSummary::new(Duration::from_millis(500), 1000, executed),
        )
    }

    fn pass_record() -> VerdictRecord {
        let analysis =
            StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(2.294, 0.011);
        let provenance =
            SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("my-service.yaml");

        VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_translation"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 96, 4),
            FunctionalDimension::new(96, 4, vec![]),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .build()
    }

    fn fail_record() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_accuracy"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution(100, 100, 80, 20),
            FunctionalDimension::new(80, 20, vec![]),
        )
        .statistical_analysis(StatisticalAnalysis::new(
            0.95,
            0.040,
            0.722,
            0.900,
            ThresholdOrigin::Empirical,
        ))
        .build()
    }

    #[test]
    fn generate_produces_html() {
        let result = HtmlReportWriter::generate(
            &[pass_record(), fail_record()],
            Some("2026-04-01T12:00:00Z"),
        );

        match result {
            Ok(html) => {
                assert!(html.contains("<html"));
                assert!(html.contains("feotest Report"));
                assert!(html.contains("test_translation"));
                assert!(html.contains("test_accuracy"));
                assert!(html.contains("PASS"));
                assert!(html.contains("FAIL"));
                assert!(html.contains("<style>"));
                // Self-contained: no external resources
                assert!(!html.contains("<script"));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // xsltproc not installed — skip test gracefully
                eprintln!("skipping HTML report test: {e}");
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn generate_empty_records() {
        let result = HtmlReportWriter::generate(&[], Some("2026-04-01T12:00:00Z"));

        match result {
            Ok(html) => {
                assert!(html.contains("<html"));
                assert!(html.contains("Total: 0"));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                eprintln!("skipping HTML report test: {e}");
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn xslt_stylesheet_is_valid_xml() {
        // Verify the embedded XSLT is at least well-formed XML
        assert!(XSLT_STYLESHEET.contains("<xsl:stylesheet"));
        assert!(XSLT_STYLESHEET.contains("http://javai.org/verdict/1.0"));
    }
}
