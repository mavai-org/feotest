//! Console verdict renderer.
//!
//! Formats a [`VerdictRecord`] into a human-readable console output following
//! the RP05 verdict output contract: header with verdict reason, body with
//! section-by-section detail, optional colour support.

use std::fmt;

use crate::latency::dimension::EvaluationStatus;
use crate::model::TerminationReason;
use crate::verdict::{Verdict, VerdictRecord};

/// Fixed label width for body section alignment.
const LABEL_WIDTH: usize = 20;

/// Renders verdict records to human-readable console output.
///
/// Supports optional ANSI colour codes. Colour is auto-detected from the
/// terminal environment but can be explicitly disabled.
pub struct ConsoleRenderer {
    colour: bool,
}

impl ConsoleRenderer {
    /// Creates a renderer with colour auto-detected from the environment.
    ///
    /// Respects the [`NO_COLOR`](https://no-color.org/) convention and the
    /// `FORCE_COLOR` override. Falls back to TTY detection on stdout.
    #[must_use]
    pub fn new() -> Self {
        Self {
            colour: detect_colour_support(),
        }
    }

    /// Creates a renderer with colour disabled.
    #[must_use]
    pub const fn without_colour() -> Self {
        Self { colour: false }
    }

    /// Renders a single verdict to the provided writer.
    ///
    /// # Errors
    ///
    /// Returns `fmt::Error` if writing fails.
    pub fn render_verdict(
        &self,
        record: &VerdictRecord,
        writer: &mut dyn fmt::Write,
    ) -> fmt::Result {
        self.render_header(record, writer)?;
        render_body(record, writer)?;
        Ok(())
    }

    /// Renders a suite summary line.
    ///
    /// # Errors
    ///
    /// Returns `fmt::Error` if writing fails.
    pub fn render_summary(
        &self,
        records: &[VerdictRecord],
        duration: std::time::Duration,
        writer: &mut dyn fmt::Write,
    ) -> fmt::Result {
        let total = records.len();
        let pass = records
            .iter()
            .filter(|r| r.verdict() == Verdict::Pass)
            .count();
        let fail = records
            .iter()
            .filter(|r| r.verdict() == Verdict::Fail)
            .count();
        let inconclusive = total - pass - fail;

        writeln!(
            writer,
            "feotest: {} tests \u{2014} {} passed, {} failed, {} inconclusive ({:.1}s)",
            total,
            pass,
            fail,
            inconclusive,
            duration.as_secs_f64()
        )
    }

    /// Convenience: renders a verdict to a `String`.
    ///
    /// # Panics
    ///
    /// Cannot panic — writing to a `String` is infallible.
    #[must_use]
    pub fn render_verdict_to_string(&self, record: &VerdictRecord) -> String {
        let mut buf = String::new();
        self.render_verdict(record, &mut buf)
            .expect("writing to String should not fail");
        buf
    }

    /// Convenience: prints a verdict to stdout.
    pub fn print_verdict(&self, record: &VerdictRecord) {
        let text = self.render_verdict_to_string(record);
        print!("{text}");
    }

    // -----------------------------------------------------------------------
    // Header
    // -----------------------------------------------------------------------

    fn render_header(&self, record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
        let verdict_label = match record.verdict() {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Inconclusive => "INCONCLUSIVE",
        };

        let title = format!(
            "\u{2550} VERDICT: {} ({}) \u{2550}",
            verdict_label,
            record.verdict_reason()
        );

        if self.colour {
            let colour = match record.verdict() {
                Verdict::Pass => "\x1b[32m",
                Verdict::Fail => "\x1b[31m",
                Verdict::Inconclusive => "\x1b[33m",
            };
            writeln!(writer, "{colour}{title}\x1b[0m")
        } else {
            writeln!(writer, "{title}")
        }
    }
}

// ---------------------------------------------------------------------------
// Body section renderers (free functions — no &self needed)
// ---------------------------------------------------------------------------

fn render_body(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    render_test_name(record, writer)?;
    writeln!(writer)?;
    render_pass_rate(record, writer)?;
    render_covariate_comparison(record, writer)?;
    render_latency_summary(record, writer)?;
    render_baseline_provenance(record, writer)?;
    render_provenance(record, writer)?;
    render_termination(record, writer)?;
    render_elapsed(record, writer)?;
    render_warnings(record, writer)?;
    Ok(())
}

fn render_test_name(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    let name = record
        .identity()
        .test_name()
        .unwrap_or_else(|| record.identity().service_contract_id());
    label_value(writer, "Test:", name)
}

fn render_pass_rate(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    let func = record.functional();
    let total = func.successes() + func.failures();
    label_value(
        writer,
        "Pass rate:",
        &format!(
            "{:.4} ({}/{} samples)",
            func.pass_rate(),
            func.successes(),
            total,
        ),
    )?;

    if let Some(analysis) = record.statistical_analysis() {
        label_value(
            writer,
            "Threshold:",
            &format!("{:.4}", analysis.threshold()),
        )?;
        label_value(
            writer,
            "Confidence:",
            &format!("{:.4}", analysis.confidence_level()),
        )?;
        label_value(
            writer,
            "Wilson lower:",
            &format!("{:.4}", analysis.wilson_lower()),
        )?;
    }

    Ok(())
}

fn render_covariate_comparison(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    let cov = record.covariate_status();
    if cov.aligned() || cov.baseline_profile().is_empty() {
        return Ok(());
    }

    writeln!(writer)?;
    writeln!(writer, "Covariate misalignment:")?;
    label_value(
        writer,
        "  Baseline:",
        &format_profile(cov.baseline_profile()),
    )?;
    label_value(
        writer,
        "  Observed:",
        &format_profile(cov.observed_profile()),
    )?;

    if !cov.misalignments().is_empty() {
        let diffs: Vec<String> = cov
            .misalignments()
            .iter()
            .map(|m| {
                format!(
                    "{} (baseline: {}, observed: {})",
                    m.key(),
                    m.baseline_value(),
                    m.observed_value()
                )
            })
            .collect();
        label_value(writer, "  Differs:", &diffs.join(", "))?;
    }

    Ok(())
}

fn render_latency_summary(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    let Some(latency) = record.latency() else {
        return Ok(());
    };

    writeln!(writer)?;
    writeln!(
        writer,
        "Latency ({} samples):",
        latency.successful_samples()
    )?;
    for ev in latency.evaluations() {
        let obs = ev.observed().map_or_else(
            || "\u{2014}".to_string(),
            |d| format!("{}ms", d.as_millis()),
        );
        let thr = format!("{}ms", ev.threshold().as_millis());
        let status = match ev.status() {
            EvaluationStatus::Pass => "PASS",
            EvaluationStatus::StrictFail => "FAIL",
            EvaluationStatus::AdvisoryWarn => "WARN",
            EvaluationStatus::Infeasible => "INFEASIBLE",
        };
        label_value(
            writer,
            &format!("  {}:", ev.percentile()),
            &format!("{obs} / {thr} [{status}]"),
        )?;
    }

    Ok(())
}

fn render_baseline_provenance(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    if let Some(bp) = record.baseline_provenance() {
        let date = if bp.generated_at().len() >= 10 {
            &bp.generated_at()[..10]
        } else {
            bp.generated_at()
        };
        writeln!(writer)?;
        label_value(writer, "Baseline:", bp.source_file())?;
        label_value(
            writer,
            "",
            &format!(
                "(measured {date}, {} samples, minPassRate={:.4})",
                bp.baseline_samples(),
                bp.derived_threshold()
            ),
        )?;
    }
    Ok(())
}

fn render_provenance(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    if let Some(prov) = record.spec_provenance() {
        if let Some(filename) = prov.spec_filename() {
            label_value(writer, "Spec:", filename)?;
        }
        label_value(writer, "Origin:", &prov.threshold_origin().to_string())?;
        if let Some(cref) = prov.contract_ref() {
            label_value(writer, "Contract:", cref)?;
        }
    }
    Ok(())
}

fn render_termination(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    let reason = record.execution().termination().reason();
    if matches!(reason, TerminationReason::Completed) {
        return Ok(());
    }

    writeln!(writer)?;
    label_value(writer, "Termination:", &reason.to_string())?;
    label_value(
        writer,
        "Executed:",
        &format!(
            "{} of {} planned",
            record.execution().samples_executed(),
            record.execution().samples_planned()
        ),
    )?;
    if let Some(detail) = record.execution().termination().detail() {
        label_value(writer, "Detail:", detail)?;
    }
    Ok(())
}

fn render_elapsed(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    let total = record.execution().cost().total_time();
    label_value(writer, "Elapsed:", &format!("{:.1}s", total.as_secs_f64()))
}

fn render_warnings(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    if record.warnings().is_empty() {
        return Ok(());
    }

    writeln!(writer)?;
    for w in record.warnings() {
        label_value(
            writer,
            "Warning:",
            &format!("[{}] {}", w.code(), w.message()),
        )?;
    }
    Ok(())
}

impl Default for ConsoleRenderer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn label_value(writer: &mut dyn fmt::Write, label: &str, value: &str) -> fmt::Result {
    writeln!(writer, "{label:<LABEL_WIDTH$} {value}")
}

fn format_profile(profile: &[(String, String)]) -> String {
    profile
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn detect_colour_support() -> bool {
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    if std::env::var("FORCE_COLOR").is_ok() {
        return true;
    }
    std::io::stdout().is_terminal()
}

// Bring the trait into scope for `is_terminal()`.
use std::io::IsTerminal;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity,
        TestIntent, ThresholdOrigin, Warning,
    };
    use crate::verdict::{
        BaselineProvenance, CovariateStatus, FunctionalDimension, Misalignment, SpecProvenance,
        StatisticalAnalysis,
    };
    use insta::assert_snapshot;
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

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
        let baseline_prov = BaselineProvenance::new(
            "my-service.yaml",
            "2026-03-27T10:00:00Z",
            200,
            0.9500,
            0.9000,
        );

        VerdictRecord::builder(
            TestIdentity::new("my-service"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 96, 4),
            FunctionalDimension::new(96, 4, vec![]),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .baseline_provenance(baseline_prov)
        .build()
    }

    fn fail_record() -> VerdictRecord {
        let analysis =
            StatisticalAnalysis::new(0.95, 0.040, 0.722, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(-1.500, 0.933);
        let provenance =
            SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("my-service.yaml");

        VerdictRecord::builder(
            TestIdentity::new("my-service"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution(100, 100, 80, 20),
            FunctionalDimension::new(
                80,
                20,
                vec![("parse".to_string(), 12), ("content".to_string(), 8)],
            ),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .build()
    }

    fn inconclusive_record() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("flaky-service"),
            Verdict::Inconclusive,
            TestIntent::Verification,
            sample_execution(10, 10, 7, 3),
            FunctionalDimension::new(7, 3, vec![]),
        )
        .build()
    }

    fn misaligned_covariate_record() -> VerdictRecord {
        let cov = CovariateStatus::new(
            false,
            vec![Misalignment::new("model", "gpt-4o", "gpt-4o-mini")],
            vec![
                ("model".to_string(), "gpt-4o".to_string()),
                ("region".to_string(), "us-east-1".to_string()),
            ],
            vec![
                ("model".to_string(), "gpt-4o-mini".to_string()),
                ("region".to_string(), "us-east-1".to_string()),
            ],
        );

        VerdictRecord::builder(
            TestIdentity::new("covariate-test"),
            Verdict::Inconclusive,
            TestIntent::Verification,
            sample_execution(100, 100, 85, 15),
            FunctionalDimension::new(85, 15, vec![]),
        )
        .covariate_status(cov)
        .build()
    }

    fn record_with_warnings() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("warned-service"),
            Verdict::Pass,
            TestIntent::Smoke,
            sample_execution(10, 10, 10, 0),
            FunctionalDimension::new(10, 0, vec![]),
        )
        .warning(Warning::new(
            "UNDERSIZED",
            "Sample size 10 is insufficient for verification-grade evidence",
        ))
        .warning(Warning::new(
            "SMOKE_NORMATIVE",
            "Smoke test against normative threshold \u{2014} verdict is not evidential",
        ))
        .build()
    }

    // -----------------------------------------------------------------------
    // Snapshot tests
    // -----------------------------------------------------------------------

    #[test]
    fn pass_verdict() {
        let record = pass_record();
        let renderer = ConsoleRenderer::without_colour();
        let output = renderer.render_verdict_to_string(&record);
        assert_snapshot!(output);
    }

    #[test]
    fn fail_verdict() {
        let record = fail_record();
        let renderer = ConsoleRenderer::without_colour();
        let output = renderer.render_verdict_to_string(&record);
        assert_snapshot!(output);
    }

    #[test]
    fn inconclusive_verdict() {
        let record = inconclusive_record();
        let renderer = ConsoleRenderer::without_colour();
        let output = renderer.render_verdict_to_string(&record);
        assert_snapshot!(output);
    }

    #[test]
    fn inconclusive_covariate_misalignment() {
        let record = misaligned_covariate_record();
        let renderer = ConsoleRenderer::without_colour();
        let output = renderer.render_verdict_to_string(&record);
        assert_snapshot!(output);
    }

    #[test]
    fn verdict_with_warnings() {
        let record = record_with_warnings();
        let renderer = ConsoleRenderer::without_colour();
        let output = renderer.render_verdict_to_string(&record);
        assert_snapshot!(output);
    }

    #[test]
    fn suite_summary() {
        let records = vec![pass_record(), fail_record(), inconclusive_record()];
        let renderer = ConsoleRenderer::without_colour();
        let mut buf = String::new();
        renderer
            .render_summary(&records, Duration::from_secs_f64(12.3), &mut buf)
            .unwrap();
        assert_snapshot!(buf);
    }

    #[test]
    fn early_termination() {
        let record = VerdictRecord::builder(
            TestIdentity::new("degraded-service"),
            Verdict::Fail,
            TestIntent::Verification,
            ExecutionSummary::new(
                100,
                42,
                20,
                22,
                TerminationInfo::new(TerminationReason::FailureInevitable),
                CostSummary::new(Duration::from_millis(200), 500, 42),
            ),
            FunctionalDimension::new(20, 22, vec![]),
        )
        .statistical_analysis(
            StatisticalAnalysis::new(0.95, 0.077, 0.342, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(-5.195, 1.000),
        )
        .spec_provenance(SpecProvenance::new(ThresholdOrigin::Empirical))
        .build();

        let renderer = ConsoleRenderer::without_colour();
        let output = renderer.render_verdict_to_string(&record);
        assert_snapshot!(output);
    }

    #[test]
    fn test_name_used_when_present() {
        let record = VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_translation_accuracy"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 96, 4),
            FunctionalDimension::new(96, 4, vec![]),
        )
        .build();

        let renderer = ConsoleRenderer::without_colour();
        let output = renderer.render_verdict_to_string(&record);
        assert!(output.contains("test_translation_accuracy"));
        assert!(!output.contains("Test:                my-service"));
    }

    #[test]
    fn verdict_reason_pass() {
        let record = pass_record();
        assert_eq!(record.verdict_reason(), "0.9600 >= 0.9000");
    }

    #[test]
    fn verdict_reason_fail() {
        let record = fail_record();
        assert_eq!(record.verdict_reason(), "0.8000 < 0.9000");
    }

    #[test]
    fn verdict_reason_inconclusive() {
        let record = inconclusive_record();
        assert_eq!(record.verdict_reason(), "insufficient evidence");
    }

    #[test]
    fn verdict_reason_covariate_misalignment() {
        let record = misaligned_covariate_record();
        assert_eq!(record.verdict_reason(), "covariate misalignment");
    }
}
