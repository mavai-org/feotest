//! `JUnit` XML output for verdict records.
//!
//! Produces JUnit-compatible XML that can be consumed by CI systems
//! and test result aggregators.

use std::io::Write;
use std::path::Path;

use crate::verdict::{Verdict, VerdictRecord};

/// Writes verdict records as `JUnit` XML.
// mavai-ref: JVI-XH8NTM1 — do not remove (resolves in mavai-orchestrator)
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
            .filter(|v| v.verdict() == Some(Verdict::Fail))
            .count();
        let errors = verdicts
            .iter()
            .filter(|v| matches!(v.verdict(), Some(Verdict::Inconclusive) | None))
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
            .unwrap_or_else(|| verdict.identity().service_contract_id());
        let classname = verdict.identity().service_contract_id();
        let time = verdict.execution().cost().total_time().as_secs_f64();

        writeln!(
            writer,
            "  <testcase name=\"{name}\" classname=\"{classname}\" time=\"{time:.3}\">"
        )?;

        match verdict.verdict() {
            Some(Verdict::Pass) => {}
            None => {
                let message = verdict.verdict_reason();
                let detail = Self::build_detail(verdict);
                writeln!(
                    writer,
                    "    <error message=\"{}\">{}</error>",
                    xml_escape(message),
                    xml_escape(&detail)
                )?;
            }
            Some(Verdict::Fail) => {
                let message = format!("Test failed: {}", verdict.verdict_reason());
                let detail = Self::build_detail(verdict);
                writeln!(
                    writer,
                    "    <failure message=\"{}\">{}</failure>",
                    xml_escape(&message),
                    xml_escape(&detail)
                )?;
            }
            Some(Verdict::Inconclusive) => {
                let message = "Test inconclusive — the data cannot decide";
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
        lines.push(format!(
            "Verdict: {}",
            verdict
                .verdict()
                .map_or_else(|| "REFUSED".to_owned(), |v| v.to_string())
        ));
        lines.push(format!("Methodology: {}", verdict.methodology_version()));
        lines.push(format!("Intent: {}", verdict.intent()));
        lines.push(format!(
            "Samples: {} / {} planned",
            exec.samples_executed(),
            exec.samples_planned()
        ));
        if let Some(func) = verdict.functional_summary() {
            lines.push(format!(
                "Pass rate: {:.4} ({}/{})",
                func.pass_rate(),
                func.pass(),
                func.total()
            ));
        }

        if let Some(stats) = verdict.statistical_analysis() {
            lines.push(format!("Rule: {}", stats.decision_rule()));
            lines.push(format!("Threshold: {:.4}", stats.threshold()));
            lines.push(format!(
                "Wilson lower [{:.0}%]: {:.4} (descriptive)",
                stats.confidence_level() * 100.0,
                stats.wilson_lower()
            ));
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
            lines.push(format!("Wilson lower: {:.4}", stats.wilson_lower()));
            lines.push(format!(
                "Threshold: {:.4} ({})",
                stats.threshold(),
                stats.threshold_origin()
            ));
            lines.push(format!("Rule: {}", stats.decision_rule()));
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
    use crate::oracle_examples::{analysis_of, compliance_row, regression_row};
    use crate::statistics::rules::ConfigurationError;
    use crate::verdict::{CriterionRow, FunctionalAssessment, VerdictRecord};
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
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
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
                80,
                20,
                TerminationInfo::new(TerminationReason::Completed),
                CostSummary::new(Duration::from_millis(500), 1000, 100),
            ),
            FunctionalAssessment::single(regression_row(
                "worked_example_fail_deep_degradation",
                "result",
            )),
        )
        .statistical_analysis(analysis_of(&regression_row(
            "worked_example_fail_deep_degradation",
            "result",
        )))
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
        assert!(xml.contains("Test failed"));
        assert!(xml.contains("Rule: regression/fisher"));
    }

    #[test]
    fn a_refused_configuration_is_an_error_naming_its_codes() {
        let record = VerdictRecord::refused(
            TestIdentity::new("refused-service"),
            TestIntent::Verification,
            ExecutionSummary::new(
                200,
                0,
                0,
                0,
                TerminationInfo::new(TerminationReason::ConfigurationRefused),
                CostSummary::new(Duration::ZERO, 0, 0),
            ),
            vec![
                ConfigurationError::ComplianceInfeasible,
                ConfigurationError::TestLargerThanBaseline,
            ],
        )
        .build();
        let mut buf = Vec::new();
        JunitXmlWriter::write_to(&mut buf, &[record]).unwrap();
        let xml = String::from_utf8(buf).unwrap();
        assert!(xml.contains("errors=\"1\""));
        assert!(
            xml.contains("configuration refused: TEST_LARGER_THAN_BASELINE COMPLIANCE_INFEASIBLE")
        );
        assert!(xml.contains("Verdict: REFUSED"));
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

    fn inconclusive_verdict() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("flaky-service"),
            Verdict::Inconclusive,
            TestIntent::Verification,
            ExecutionSummary::new(
                100,
                100,
                60,
                40,
                TerminationInfo::new(TerminationReason::Completed),
                CostSummary::new(Duration::from_millis(500), 1000, 100),
            ),
            FunctionalAssessment::single(CriterionRow::result(
                60,
                40,
                vec![],
                Verdict::Inconclusive,
            )),
        )
        .build()
    }

    #[test]
    fn writes_error_element_for_inconclusive_test() {
        let mut buf = Vec::new();
        JunitXmlWriter::write_to(&mut buf, &[inconclusive_verdict()]).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("errors=\"1\""));
        assert!(xml.contains("<error"));
        assert!(xml.contains("inconclusive"));
    }

    #[test]
    fn system_out_includes_statistical_details() {
        let row = compliance_row("p95_n150_pass_at_k_min", "result", ThresholdOrigin::Sla);
        let analysis = analysis_of(&row);

        let provenance = crate::verdict::SpecProvenance::new(ThresholdOrigin::Empirical)
            .with_spec_filename("my-service.yaml")
            .with_contract_ref("SLA v2.0");

        let record = VerdictRecord::builder(
            TestIdentity::new("my-service"),
            Verdict::Pass,
            TestIntent::Verification,
            ExecutionSummary::new(
                150,
                150,
                row.pass(),
                row.fail(),
                TerminationInfo::new(TerminationReason::Completed),
                CostSummary::new(Duration::from_millis(500), 1000, 150),
            ),
            FunctionalAssessment::single(row),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .build();

        let mut buf = Vec::new();
        JunitXmlWriter::write_to(&mut buf, &[record]).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("<system-out>"));
        assert!(xml.contains("Confidence:"));
        assert!(xml.contains("Baseline: my-service.yaml"));
        assert!(xml.contains("Contract: SLA v2.0"));
        assert!(xml.contains("Rule: compliance/exact-binomial"));
    }

    #[test]
    fn mixed_suite_counts_correctly() {
        let mut buf = Vec::new();
        JunitXmlWriter::write_to(
            &mut buf,
            &[pass_verdict(), fail_verdict(), inconclusive_verdict()],
        )
        .unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("tests=\"3\""));
        assert!(xml.contains("failures=\"1\""));
        assert!(xml.contains("errors=\"1\""));
    }

    #[test]
    fn fail_detail_includes_warnings() {
        let record = VerdictRecord::builder(
            TestIdentity::new("warned"),
            Verdict::Fail,
            TestIntent::Smoke,
            ExecutionSummary::new(
                10,
                10,
                5,
                5,
                TerminationInfo::new(TerminationReason::Completed),
                CostSummary::new(Duration::from_millis(100), 0, 10),
            ),
            FunctionalAssessment::single(CriterionRow::result(5, 5, vec![], Verdict::Fail)),
        )
        .warning(crate::model::Warning::new("UNDERSIZED", "too small"))
        .build();

        let mut buf = Vec::new();
        JunitXmlWriter::write_to(&mut buf, &[record]).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("Warning:"));
        assert!(xml.contains("UNDERSIZED"));
    }

    #[test]
    fn uses_service_contract_id_when_no_test_name() {
        let record = VerdictRecord::builder(
            TestIdentity::new("no-name-service"),
            Verdict::Pass,
            TestIntent::Verification,
            ExecutionSummary::new(
                10,
                10,
                10,
                0,
                TerminationInfo::new(TerminationReason::Completed),
                CostSummary::new(Duration::from_millis(50), 0, 10),
            ),
            FunctionalAssessment::single(CriterionRow::result(10, 0, vec![], Verdict::Pass)),
        )
        .build();

        let mut buf = Vec::new();
        JunitXmlWriter::write_to(&mut buf, &[record]).unwrap();
        let xml = String::from_utf8(buf).unwrap();

        assert!(xml.contains("name=\"no-name-service\""));
    }
}
