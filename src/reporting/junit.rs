//! `JUnit` XML output for verdict records.
//!
//! Produces JUnit-compatible XML that can be consumed by CI systems
//! and test result aggregators.

use std::io::Write;
use std::path::Path;

use crate::verdict::{Verdict, VerdictRecord};

/// Writes verdict records as `JUnit` XML.
pub struct JunitXmlWriter;

impl JunitXmlWriter {
    /// Writes a collection of verdict records as a `JUnit` XML test suite.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_to<W: Write>(writer: &mut W, verdicts: &[VerdictRecord]) -> std::io::Result<()> {
        let tests = verdicts.len();
        let failures = verdicts
            .iter()
            .filter(|v| v.verdict() == Verdict::Fail)
            .count();
        let errors = verdicts
            .iter()
            .filter(|v| v.verdict() == Verdict::Inconclusive)
            .count();

        let total_time_secs: f64 = verdicts
            .iter()
            .map(|v| v.execution().cost().total_time().as_secs_f64())
            .sum();

        writeln!(writer, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
        writeln!(
            writer,
            "<testsuite name=\"feotest\" tests=\"{tests}\" failures=\"{failures}\" errors=\"{errors}\" time=\"{total_time_secs:.3}\">"
        )?;

        for verdict in verdicts {
            Self::write_testcase(writer, verdict)?;
        }

        writeln!(writer, "</testsuite>")?;
        Ok(())
    }

    /// Writes verdict records to a file.
    ///
    /// Creates parent directories if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be created or written.
    pub fn write_to_file(path: &Path, verdicts: &[VerdictRecord]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(path)?;
        Self::write_to(&mut file, verdicts)
    }

    fn write_testcase<W: Write>(writer: &mut W, verdict: &VerdictRecord) -> std::io::Result<()> {
        let name = verdict
            .identity()
            .test_name()
            .unwrap_or_else(|| verdict.identity().use_case_id());
        let classname = verdict.identity().use_case_id();
        let time = verdict.execution().cost().total_time().as_secs_f64();

        writeln!(
            writer,
            "  <testcase name=\"{name}\" classname=\"{classname}\" time=\"{time:.3}\">"
        )?;

        match verdict.verdict() {
            Verdict::Pass => {}
            Verdict::Fail => {
                let message = format!(
                    "Observed pass rate {:.4} below threshold",
                    verdict.functional().pass_rate()
                );
                let detail = Self::build_detail(verdict);
                writeln!(
                    writer,
                    "    <failure message=\"{}\">{}</failure>",
                    xml_escape(&message),
                    xml_escape(&detail)
                )?;
            }
            Verdict::Inconclusive => {
                let message = "Test inconclusive — statistical analysis unreliable";
                let detail = Self::build_detail(verdict);
                writeln!(
                    writer,
                    "    <error message=\"{message}\">{}</error>",
                    xml_escape(&detail)
                )?;
            }
        }

        // System output: statistical details
        let stdout = Self::build_system_out(verdict);
        if !stdout.is_empty() {
            writeln!(
                writer,
                "    <system-out>{}</system-out>",
                xml_escape(&stdout)
            )?;
        }

        writeln!(writer, "  </testcase>")?;
        Ok(())
    }

    fn build_detail(verdict: &VerdictRecord) -> String {
        let mut lines = Vec::new();
        let exec = verdict.execution();
        let func = verdict.functional();

        lines.push(format!("Verdict: {}", verdict.verdict()));
        lines.push(format!("Intent: {}", verdict.intent()));
        lines.push(format!(
            "Samples: {} / {} planned",
            exec.samples_executed(),
            exec.samples_planned()
        ));
        lines.push(format!(
            "Pass rate: {:.4} ({}/{})",
            func.pass_rate(),
            func.successes(),
            func.successes() + func.failures()
        ));

        if let Some(stats) = verdict.statistical_analysis() {
            lines.push(format!("Threshold: {:.4}", stats.threshold()));
            lines.push(format!(
                "CI 95%: [{:.4}, {:.4}]",
                stats.ci_lower(),
                stats.ci_upper()
            ));
            if let Some(p) = stats.p_value() {
                lines.push(format!("p-value: {p:.4}"));
            }
        }

        for warning in verdict.warnings() {
            lines.push(format!("Warning: {warning}"));
        }

        lines.join("\n")
    }

    fn build_system_out(verdict: &VerdictRecord) -> String {
        let mut lines = Vec::new();

        if let Some(stats) = verdict.statistical_analysis() {
            lines.push(format!(
                "Confidence: {:.2}%",
                stats.confidence_level() * 100.0
            ));
            lines.push(format!("SE: {:.4}", stats.standard_error()));
            lines.push(format!(
                "CI: [{:.4}, {:.4}]",
                stats.ci_lower(),
                stats.ci_upper()
            ));
            lines.push(format!(
                "Threshold: {:.4} ({})",
                stats.threshold(),
                stats.threshold_origin()
            ));
            if let Some(z) = stats.test_statistic() {
                lines.push(format!("z: {z:.4}"));
            }
            if let Some(p) = stats.p_value() {
                lines.push(format!("p: {p:.4}"));
            }
        }

        if let Some(prov) = verdict.spec_provenance() {
            if let Some(file) = prov.spec_filename() {
                lines.push(format!("Baseline: {file}"));
            }
            if let Some(contract) = prov.contract_ref() {
                lines.push(format!("Contract: {contract}"));
            }
        }

        lines.join("\n")
    }
}

/// Escapes special XML characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity,
        TestIntent, ThresholdOrigin,
    };
    use crate::verdict::{FunctionalDimension, StatisticalAnalysis, VerdictRecord};
    use std::time::Duration;

    fn pass_verdict() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("shopping-basket").with_test_name("test_translation"),
            Verdict::Pass,
            TestIntent::Verification,
            ExecutionSummary::new(
                100,
                100,
                95,
                5,
                TerminationInfo::new(TerminationReason::Completed),
                CostSummary::new(Duration::from_millis(500), 1000, 100),
            ),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .build()
    }

    fn fail_verdict() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("shopping-basket").with_test_name("test_translation"),
            Verdict::Fail,
            TestIntent::Verification,
            ExecutionSummary::new(
                100,
                100,
                70,
                30,
                TerminationInfo::new(TerminationReason::Completed),
                CostSummary::new(Duration::from_millis(500), 1000, 100),
            ),
            FunctionalDimension::new(70, 30, vec![]),
        )
        .statistical_analysis(StatisticalAnalysis::new(
            0.95,
            0.0458,
            0.6071,
            0.7929,
            0.80,
            ThresholdOrigin::Empirical,
        ))
        .build()
    }

    #[test]
    fn writes_valid_xml_for_passing_suite() {
        let mut buf = Vec::new();
        JunitXmlWriter::write_to(&mut buf, &[pass_verdict()]).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("<?xml version=\"1.0\""));
        assert!(xml.contains("tests=\"1\""));
        assert!(xml.contains("failures=\"0\""));
        assert!(xml.contains("name=\"test_translation\""));
        assert!(xml.contains("classname=\"shopping-basket\""));
        assert!(xml.contains("</testsuite>"));
    }

    #[test]
    fn writes_failure_element_for_failing_test() {
        let mut buf = Vec::new();
        JunitXmlWriter::write_to(&mut buf, &[fail_verdict()]).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("failures=\"1\""));
        assert!(xml.contains("<failure"));
        assert!(xml.contains("Observed pass rate"));
    }

    #[test]
    fn writes_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.xml");

        JunitXmlWriter::write_to_file(&path, &[pass_verdict()]).unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("testsuite"));
    }

    #[test]
    fn escapes_xml_characters() {
        assert_eq!(
            xml_escape("<test & \"quotes\">"),
            "&lt;test &amp; &quot;quotes&quot;&gt;"
        );
    }

    #[test]
    fn empty_verdicts_produce_empty_suite() {
        let mut buf = Vec::new();
        JunitXmlWriter::write_to(&mut buf, &[]).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("tests=\"0\""));
        assert!(xml.contains("failures=\"0\""));
    }
}
