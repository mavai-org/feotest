//! Verdict XML serialisation (RP07 interchange format).
//!
//! Serialises a [`VerdictRecord`] to XML conforming to the
//! `http://javai.org/verdict/1.0` schema. The output is a standalone
//! `<verdict-record>` document suitable for file-per-test persistence
//! and XSLT transformation to HTML.

use std::fmt::Write;
use std::io;
use std::path::Path;

use crate::latency::dimension::EvaluationStatus;
use crate::latency::resolver::ThresholdProvenance;
use crate::verdict::{Verdict, VerdictRecord};

/// The XML namespace for the verdict interchange format.
const NAMESPACE: &str = "http://javai.org/verdict/1.0";

/// The schema version.
const VERSION: &str = "1.0";

/// Serialises verdict records to the RP07 verdict XML interchange format.
pub struct VerdictXmlWriter;

impl VerdictXmlWriter {
    /// Serialises a single verdict record as a complete XML document.
    ///
    /// The `timestamp` parameter is an ISO 8601 string for the
    /// `timestamp` attribute on the root element. When `None`, the
    /// attribute is omitted.
    #[must_use]
    pub fn write_record(record: &VerdictRecord, timestamp: Option<&str>) -> String {
        let mut xml = String::with_capacity(4096);

        writeln!(xml, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>").unwrap();
        write!(xml, "<verdict-record xmlns=\"{NAMESPACE}\"").unwrap();
        write!(xml, " version=\"{VERSION}\"").unwrap();
        if let Some(ts) = timestamp {
            write!(xml, " timestamp=\"{}\"", escape_attr(ts)).unwrap();
        }
        if let Some(id) = record.correlation_id() {
            write!(xml, " correlation-id=\"{}\"", escape_attr(id)).unwrap();
        }
        writeln!(xml, " generator=\"feotest/{}\">", env!("CARGO_PKG_VERSION")).unwrap();

        write_identity(&mut xml, record);
        write_execution(&mut xml, record);
        write_functional(&mut xml, record);
        write_latency(&mut xml, record);
        write_statistics(&mut xml, record);
        write_covariates(&mut xml, record);
        write_provenance(&mut xml, record);
        write_baseline(&mut xml, record);
        write_termination(&mut xml, record);
        write_cost(&mut xml, record);
        write_warnings(&mut xml, record);
        write_pacing(&mut xml, record);
        write_environment(&mut xml, record);
        write_verdict(&mut xml, record);

        writeln!(xml, "</verdict-record>").unwrap();
        xml
    }

    /// Wraps multiple verdict XML fragments in a `<verdict-suite>` envelope.
    ///
    /// Each element of `record_xmls` should be a complete
    /// `<verdict-record>` document (including the XML declaration).
    /// The declaration is stripped before wrapping.
    #[must_use]
    pub fn wrap_suite(record_xmls: &[String], timestamp: Option<&str>) -> String {
        let mut xml =
            String::with_capacity(record_xmls.iter().map(String::len).sum::<usize>() + 256);

        writeln!(xml, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>").unwrap();
        write!(xml, "<verdict-suite xmlns=\"{NAMESPACE}\"").unwrap();
        if let Some(ts) = timestamp {
            write!(xml, " timestamp=\"{}\"", escape_attr(ts)).unwrap();
        }
        writeln!(xml, ">").unwrap();

        for record_xml in record_xmls {
            // Strip the XML declaration from each record before embedding.
            let content = record_xml
                .strip_prefix("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")
                .unwrap_or(record_xml);
            writeln!(xml, "{content}").unwrap();
        }

        writeln!(xml, "</verdict-suite>").unwrap();
        xml
    }

    /// Writes a verdict record to a file.
    ///
    /// Creates parent directories if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be created or written.
    pub fn write_to_file(
        path: &Path,
        record: &VerdictRecord,
        timestamp: Option<&str>,
    ) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let xml = Self::write_record(record, timestamp);
        std::fs::write(path, xml)
    }
}

// ---------------------------------------------------------------------------
// Element writers
// ---------------------------------------------------------------------------

fn write_identity(w: &mut String, record: &VerdictRecord) {
    let id = record.identity();
    write!(
        w,
        "  <identity use-case-id=\"{}\"",
        escape_attr(id.service_contract_id())
    )
    .unwrap();
    if let Some(name) = id.test_name() {
        write!(w, " test-name=\"{}\"", escape_attr(name)).unwrap();
    }
    writeln!(w, "/>").unwrap();
}

fn write_execution(w: &mut String, record: &VerdictRecord) {
    let exec = record.execution();
    write!(w, "  <execution").unwrap();
    write!(w, " planned-samples=\"{}\"", exec.samples_planned()).unwrap();
    write!(w, " samples-executed=\"{}\"", exec.samples_executed()).unwrap();
    write!(w, " successes=\"{}\"", exec.successes()).unwrap();
    write!(w, " failures=\"{}\"", exec.failures()).unwrap();
    write!(
        w,
        " elapsed-ms=\"{}\"",
        exec.cost().total_time().as_millis()
    )
    .unwrap();
    write!(w, " intent=\"{}\"", record.intent()).unwrap();
    if let Some(stats) = record.statistical_analysis() {
        write!(w, " confidence=\"{}\"", stats.confidence_level()).unwrap();
    }
    writeln!(w, "/>").unwrap();
}

fn write_functional(w: &mut String, record: &VerdictRecord) {
    let func = record.functional_summary();
    let has_distribution = !func.failure_distribution().is_empty();

    write!(w, "  <functional").unwrap();
    write!(w, " successes=\"{}\"", func.pass()).unwrap();
    write!(w, " failures=\"{}\"", func.fail()).unwrap();
    write!(w, " pass-rate=\"{:.4}\"", func.pass_rate()).unwrap();

    if has_distribution {
        writeln!(w, ">").unwrap();
        writeln!(w, "    <failure-distribution>").unwrap();
        for (name, count) in func.failure_distribution() {
            writeln!(
                w,
                "      <check name=\"{}\" count=\"{count}\"/>",
                escape_attr(name)
            )
            .unwrap();
        }
        writeln!(w, "    </failure-distribution>").unwrap();
        writeln!(w, "  </functional>").unwrap();
    } else {
        writeln!(w, "/>").unwrap();
    }
}

fn write_latency(w: &mut String, record: &VerdictRecord) {
    let Some(latency) = record.latency() else {
        return;
    };

    write!(w, "  <latency").unwrap();
    write!(
        w,
        " successful-samples=\"{}\"",
        latency.successful_samples()
    )
    .unwrap();
    write!(w, " strict-violations=\"{}\"", latency.strict_violations()).unwrap();
    write!(
        w,
        " advisory-violations=\"{}\"",
        latency.advisory_violations()
    )
    .unwrap();
    writeln!(w, ">").unwrap();

    // Observed percentiles
    if !latency.observed_percentiles().is_empty() {
        writeln!(w, "    <observed>").unwrap();
        for (percentile, duration) in latency.observed_percentiles() {
            writeln!(
                w,
                "      <percentile label=\"{}\" value-ms=\"{}\"/>",
                percentile.label(),
                duration.as_millis()
            )
            .unwrap();
        }
        writeln!(w, "    </observed>").unwrap();
    }

    // Evaluations
    if !latency.evaluations().is_empty() {
        writeln!(w, "    <evaluations>").unwrap();
        for ev in latency.evaluations() {
            write!(
                w,
                "      <evaluation percentile=\"{}\"",
                ev.percentile().label()
            )
            .unwrap();
            if let Some(obs) = ev.observed() {
                write!(w, " observed-ms=\"{}\"", obs.as_millis()).unwrap();
            }
            write!(w, " threshold-ms=\"{}\"", ev.threshold().as_millis()).unwrap();

            match ev.provenance() {
                ThresholdProvenance::Explicit => {
                    write!(w, " provenance=\"explicit\"").unwrap();
                }
                ThresholdProvenance::BaselineDerived {
                    confidence,
                    rank,
                    n,
                } => {
                    write!(w, " provenance=\"baseline-derived\"").unwrap();
                    write!(w, " baseline-confidence=\"{confidence:.2}\"").unwrap();
                    write!(w, " baseline-rank=\"{rank}\"").unwrap();
                    write!(w, " baseline-n=\"{n}\"").unwrap();
                }
            }

            let mode = match ev.mode() {
                crate::latency::enforcement::LatencyEnforcementMode::Advisory => "advisory",
                crate::latency::enforcement::LatencyEnforcementMode::Strict => "strict",
            };
            write!(w, " mode=\"{mode}\"").unwrap();

            let status = match ev.status() {
                EvaluationStatus::Pass => "PASS",
                EvaluationStatus::StrictFail => "STRICT_FAIL",
                EvaluationStatus::AdvisoryWarn => "ADVISORY_WARN",
                EvaluationStatus::Infeasible => "INFEASIBLE",
            };
            writeln!(w, " status=\"{status}\"/>").unwrap();
        }
        writeln!(w, "    </evaluations>").unwrap();
    }

    writeln!(w, "  </latency>").unwrap();
}

fn write_statistics(w: &mut String, record: &VerdictRecord) {
    let Some(stats) = record.statistical_analysis() else {
        return;
    };

    write!(w, "  <statistics").unwrap();
    write!(w, " confidence-level=\"{:.4}\"", stats.confidence_level()).unwrap();
    write!(w, " standard-error=\"{:.4}\"", stats.standard_error()).unwrap();
    write!(w, " wilson-lower=\"{:.4}\"", stats.wilson_lower()).unwrap();
    write!(w, " threshold=\"{:.4}\"", stats.threshold()).unwrap();
    write!(w, " threshold-origin=\"{}\"", stats.threshold_origin()).unwrap();
    if let Some(z) = stats.test_statistic() {
        write!(w, " test-statistic=\"{z:.4}\"").unwrap();
    }
    if let Some(p) = stats.p_value() {
        write!(w, " p-value=\"{p:.4}\"").unwrap();
    }
    writeln!(w, "/>").unwrap();
}

fn write_covariates(w: &mut String, record: &VerdictRecord) {
    let cov = record.covariate_status();

    if cov.aligned() && cov.misalignments().is_empty() {
        writeln!(w, "  <covariates aligned=\"true\"/>").unwrap();
        return;
    }

    writeln!(
        w,
        "  <covariates aligned=\"{}\">",
        if cov.aligned() { "true" } else { "false" }
    )
    .unwrap();
    for m in cov.misalignments() {
        writeln!(
            w,
            "    <misalignment key=\"{}\" baseline-value=\"{}\" observed-value=\"{}\"/>",
            escape_attr(m.key()),
            escape_attr(m.baseline_value()),
            escape_attr(m.observed_value())
        )
        .unwrap();
    }
    writeln!(w, "  </covariates>").unwrap();
}

fn write_provenance(w: &mut String, record: &VerdictRecord) {
    let Some(prov) = record.spec_provenance() else {
        return;
    };

    write!(w, "  <provenance origin=\"{}\"", prov.threshold_origin()).unwrap();
    if let Some(file) = prov.spec_filename() {
        write!(w, " spec-filename=\"{}\"", escape_attr(file)).unwrap();
    }
    if let Some(cref) = prov.contract_ref() {
        write!(w, " contract-ref=\"{}\"", escape_attr(cref)).unwrap();
    }

    if let Some(exp) = prov.expiration() {
        writeln!(w, ">").unwrap();
        write!(w, "    <expiration status=\"{}\"", exp.status().xml_name()).unwrap();
        if let Some(at) = exp.expires_at() {
            write!(w, " expires-at=\"{}\"", escape_attr(at)).unwrap();
        }
        write!(
            w,
            " requires-warning=\"{}\"",
            exp.status().requires_warning()
        )
        .unwrap();
        writeln!(w, "/>").unwrap();
        writeln!(w, "  </provenance>").unwrap();
    } else {
        writeln!(w, "/>").unwrap();
    }
}

fn write_baseline(w: &mut String, record: &VerdictRecord) {
    let Some(bp) = record.baseline_provenance() else {
        return;
    };

    write!(w, "  <baseline").unwrap();
    write!(w, " source-file=\"{}\"", escape_attr(bp.source_file())).unwrap();
    write!(w, " generated-at=\"{}\"", escape_attr(bp.generated_at())).unwrap();
    write!(w, " samples=\"{}\"", bp.baseline_samples()).unwrap();
    write!(w, " baseline-rate=\"{:.4}\"", bp.baseline_rate()).unwrap();
    write!(w, " derived-threshold=\"{:.4}\"", bp.derived_threshold()).unwrap();
    writeln!(w, "/>").unwrap();
}

fn write_termination(w: &mut String, record: &VerdictRecord) {
    let term = record.execution().termination();
    write!(w, "  <termination reason=\"{}\"", term.reason()).unwrap();
    if let Some(detail) = term.detail() {
        write!(w, " detail=\"{}\"", escape_attr(detail)).unwrap();
    }
    writeln!(w, "/>").unwrap();
}

fn write_cost(w: &mut String, record: &VerdictRecord) {
    let cost = record.execution().cost();
    write!(w, "  <cost").unwrap();
    write!(w, " total-time-ms=\"{}\"", cost.total_time().as_millis()).unwrap();
    write!(w, " total-tokens=\"{}\"", cost.total_tokens()).unwrap();
    write!(
        w,
        " avg-time-per-sample-ms=\"{}\"",
        cost.avg_time_per_sample().as_millis()
    )
    .unwrap();
    write!(
        w,
        " avg-tokens-per-sample=\"{}\"",
        cost.avg_tokens_per_sample()
    )
    .unwrap();
    writeln!(w, "/>").unwrap();
}

fn write_warnings(w: &mut String, record: &VerdictRecord) {
    if record.warnings().is_empty() {
        return;
    }

    writeln!(w, "  <warnings>").unwrap();
    for warning in record.warnings() {
        writeln!(
            w,
            "    <warning code=\"{}\">{}</warning>",
            escape_attr(warning.code()),
            escape_text(warning.message())
        )
        .unwrap();
    }
    writeln!(w, "  </warnings>").unwrap();
}

fn write_pacing(w: &mut String, record: &VerdictRecord) {
    let Some(pacing) = record.pacing() else {
        return;
    };
    write!(w, "  <pacing").unwrap();
    write!(w, " max-rps=\"{}\"", pacing.max_rps()).unwrap();
    write!(w, " max-rpm=\"{}\"", pacing.max_rpm()).unwrap();
    write!(w, " max-concurrent=\"{}\"", pacing.max_concurrent()).unwrap();
    write!(
        w,
        " effective-min-delay-ms=\"{}\"",
        pacing.effective_min_delay_ms()
    )
    .unwrap();
    write!(
        w,
        " effective-concurrency=\"{}\"",
        pacing.effective_concurrency()
    )
    .unwrap();
    write!(w, " effective-rps=\"{}\"", pacing.effective_rps()).unwrap();
    writeln!(w, "/>").unwrap();
}

fn write_environment(w: &mut String, record: &VerdictRecord) {
    if record.environment().is_empty() {
        return;
    }
    writeln!(w, "  <environment>").unwrap();
    for (key, value) in record.environment() {
        writeln!(
            w,
            "    <entry key=\"{}\" value=\"{}\"/>",
            escape_attr(key),
            escape_attr(value)
        )
        .unwrap();
    }
    writeln!(w, "  </environment>").unwrap();
}

fn write_verdict(w: &mut String, record: &VerdictRecord) {
    let verdict_str = match record.verdict() {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::Inconclusive => "INCONCLUSIVE",
    };

    write!(w, "  <verdict value=\"{verdict_str}\"").unwrap();
    let reason = record.verdict_reason();
    if !reason.is_empty() {
        write!(w, " reason=\"{}\"", escape_attr(reason)).unwrap();
    }
    writeln!(w, "/>").unwrap();
}

// ---------------------------------------------------------------------------
// XML escaping
// ---------------------------------------------------------------------------

/// Escapes special characters for use in XML attribute values.
fn escape_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Escapes special characters for use in XML text content.
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::PacingConfig;
    use crate::latency::dimension::{EvaluationStatus, LatencyDimension, LatencyEvaluation};
    use crate::latency::enforcement::LatencyEnforcementMode;
    use crate::latency::percentile::Percentile;
    use crate::latency::resolver::ThresholdProvenance;
    use crate::model::{
        CostSummary, ExecutionSummary, ExpirationInfo, ExpirationStatus, PacingSummary,
        TerminationInfo, TerminationReason, TestIdentity, TestIntent, ThresholdOrigin, Warning,
    };
    use crate::verdict::{
        BaselineProvenance, CovariateStatus, CriterionRow, FunctionalAssessment, Misalignment,
        SpecProvenance, StatisticalAnalysis,
    };
    use insta::assert_snapshot;
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
        let baseline_prov = BaselineProvenance::new(
            "my-service.yaml",
            "2026-03-27T10:00:00Z",
            200,
            0.9500,
            0.9000,
        );

        VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_translation"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 96, 4),
            FunctionalAssessment::single(CriterionRow::result(96, 4, vec![], Verdict::Pass)),
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

        VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_accuracy"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution(100, 100, 80, 20),
            FunctionalAssessment::single(CriterionRow::result(
                80,
                20,
                vec![("parse".to_string(), 12), ("content".to_string(), 8)],
                Verdict::Fail,
            )),
        )
        .statistical_analysis(analysis)
        .build()
    }

    fn inconclusive_covariate_record() -> VerdictRecord {
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
            FunctionalAssessment::single(CriterionRow::result(
                85,
                15,
                vec![],
                Verdict::Inconclusive,
            )),
        )
        .covariate_status(cov)
        .build()
    }

    fn record_with_latency() -> VerdictRecord {
        let analysis =
            StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(2.294, 0.011);

        let latency = LatencyDimension::from_parts(
            vec![
                (Percentile::P50, Duration::from_millis(120)),
                (Percentile::P95, Duration::from_millis(450)),
                (Percentile::P99, Duration::from_millis(890)),
            ],
            vec![
                LatencyEvaluation::new(
                    Percentile::P50,
                    Some(Duration::from_millis(120)),
                    Duration::from_millis(200),
                    ThresholdProvenance::Explicit,
                    LatencyEnforcementMode::Advisory,
                    EvaluationStatus::Pass,
                ),
                LatencyEvaluation::new(
                    Percentile::P95,
                    Some(Duration::from_millis(450)),
                    Duration::from_millis(500),
                    ThresholdProvenance::Explicit,
                    LatencyEnforcementMode::Advisory,
                    EvaluationStatus::Pass,
                ),
                LatencyEvaluation::new(
                    Percentile::P99,
                    Some(Duration::from_millis(890)),
                    Duration::from_millis(800),
                    ThresholdProvenance::Explicit,
                    LatencyEnforcementMode::Advisory,
                    EvaluationStatus::AdvisoryWarn,
                ),
            ],
            0,
            1,
            95,
        );

        VerdictRecord::builder(
            TestIdentity::new("latency-service").with_test_name("test_response_time"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 95, 5),
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
        )
        .statistical_analysis(analysis)
        .latency(latency)
        .build()
    }

    fn record_with_warnings() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("warned-service"),
            Verdict::Pass,
            TestIntent::Smoke,
            sample_execution(10, 10, 10, 0),
            FunctionalAssessment::single(CriterionRow::result(10, 0, vec![], Verdict::Pass)),
        )
        .warning(Warning::new(
            "UNDERSIZED",
            "Sample size 10 is insufficient for verification-grade evidence",
        ))
        .warning(Warning::new(
            "SMOKE_NORMATIVE",
            "Smoke test against normative threshold",
        ))
        .build()
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn escape_attr_handles_special_characters() {
        assert_eq!(escape_attr("a & b"), "a &amp; b");
        assert_eq!(escape_attr("<script>"), "&lt;script&gt;");
        assert_eq!(escape_attr("he said \"hi\""), "he said &quot;hi&quot;");
        assert_eq!(escape_attr("it's"), "it&apos;s");
    }

    #[test]
    fn escape_text_handles_special_characters() {
        assert_eq!(escape_text("a & b"), "a &amp; b");
        assert_eq!(escape_text("<tag>"), "&lt;tag&gt;");
        // Quotes are not escaped in text content
        assert_eq!(escape_text("he said \"hi\""), "he said \"hi\"");
    }

    #[test]
    fn xml_starts_with_declaration() {
        let xml = VerdictXmlWriter::write_record(&pass_record(), Some("2026-04-01T12:00:00Z"));
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    }

    #[test]
    fn xml_contains_namespace() {
        let xml = VerdictXmlWriter::write_record(&pass_record(), Some("2026-04-01T12:00:00Z"));
        assert!(xml.contains("xmlns=\"http://javai.org/verdict/1.0\""));
    }

    #[test]
    fn xml_contains_generator() {
        let xml = VerdictXmlWriter::write_record(&pass_record(), Some("2026-04-01T12:00:00Z"));
        assert!(xml.contains("generator=\"feotest/"));
    }

    // -----------------------------------------------------------------------
    // Snapshot tests
    // -----------------------------------------------------------------------

    #[test]
    fn xml_pass_record() {
        let xml = VerdictXmlWriter::write_record(&pass_record(), Some("2026-04-01T12:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_fail_record() {
        let xml = VerdictXmlWriter::write_record(&fail_record(), Some("2026-04-01T12:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_inconclusive_covariate() {
        let xml = VerdictXmlWriter::write_record(
            &inconclusive_covariate_record(),
            Some("2026-04-01T12:00:00Z"),
        );
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_with_latency() {
        let xml =
            VerdictXmlWriter::write_record(&record_with_latency(), Some("2026-04-01T12:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_with_warnings() {
        let xml =
            VerdictXmlWriter::write_record(&record_with_warnings(), Some("2026-04-01T12:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_suite_envelope() {
        let xml1 = VerdictXmlWriter::write_record(&pass_record(), Some("2026-04-01T12:00:00Z"));
        let xml2 = VerdictXmlWriter::write_record(&fail_record(), Some("2026-04-01T12:00:00Z"));
        let suite = VerdictXmlWriter::wrap_suite(&[xml1, xml2], Some("2026-04-01T12:00:00Z"));
        assert_snapshot!(suite);
    }

    #[test]
    fn xml_minimal_record() {
        let record = VerdictRecord::builder(
            TestIdentity::new("minimal"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(10, 10, 10, 0),
            FunctionalAssessment::single(CriterionRow::result(10, 0, vec![], Verdict::Pass)),
        )
        .build();
        let xml = VerdictXmlWriter::write_record(&record, Some("2026-04-01T12:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_with_correlation_id() {
        let record = VerdictRecord::builder(
            TestIdentity::new("correlated-service"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(50, 50, 48, 2),
            FunctionalAssessment::single(CriterionRow::result(48, 2, vec![], Verdict::Pass)),
        )
        .correlation_id("run-20260419-abc123")
        .build();
        let xml = VerdictXmlWriter::write_record(&record, Some("2026-04-19T10:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_with_pacing() {
        let pacing_config = PacingConfig::new()
            .max_requests_per_second(10.0)
            .max_requests_per_minute(300.0);
        let pacing_summary = PacingSummary::from_config(&pacing_config);

        let record = VerdictRecord::builder(
            TestIdentity::new("paced-service"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 95, 5),
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
        )
        .pacing(pacing_summary)
        .build();
        let xml = VerdictXmlWriter::write_record(&record, Some("2026-04-19T10:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_with_environment() {
        let record = VerdictRecord::builder(
            TestIdentity::new("env-service"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(50, 50, 50, 0),
            FunctionalAssessment::single(CriterionRow::result(50, 0, vec![], Verdict::Pass)),
        )
        .environment(vec![
            ("cloud_provider".to_string(), "aws".to_string()),
            ("region".to_string(), "eu-west-1".to_string()),
            ("instance_type".to_string(), "m5.large".to_string()),
        ])
        .build();
        let xml = VerdictXmlWriter::write_record(&record, Some("2026-04-19T10:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_with_expiration() {
        let provenance = SpecProvenance::new(ThresholdOrigin::Empirical)
            .with_spec_filename("aging-service.yaml")
            .with_expiration(ExpirationInfo::new(
                ExpirationStatus::ExpiringSoon,
                Some("2026-05-01T00:00:00Z".into()),
            ));

        let record = VerdictRecord::builder(
            TestIdentity::new("aging-service"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 92, 8),
            FunctionalAssessment::single(CriterionRow::result(92, 8, vec![], Verdict::Pass)),
        )
        .spec_provenance(provenance)
        .build();
        let xml = VerdictXmlWriter::write_record(&record, Some("2026-04-19T10:00:00Z"));
        assert_snapshot!(xml);
    }

    #[test]
    fn xml_full_record() {
        let pacing_config = PacingConfig::new().max_requests_per_second(5.0);
        let pacing_summary = PacingSummary::from_config(&pacing_config);

        let analysis =
            StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(2.294, 0.011);

        let provenance = SpecProvenance::new(ThresholdOrigin::Empirical)
            .with_spec_filename("full-service.yaml")
            .with_contract_ref("SLA v2.0 §3")
            .with_expiration(ExpirationInfo::new(
                ExpirationStatus::Valid,
                Some("2026-12-31T23:59:59Z".into()),
            ));

        let baseline_prov = BaselineProvenance::new(
            "full-service.yaml",
            "2026-03-01T08:00:00Z",
            500,
            0.9600,
            0.9200,
        );

        let record = VerdictRecord::builder(
            TestIdentity::new("full-service").with_test_name("test_everything"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(200, 200, 192, 8),
            FunctionalAssessment::single(CriterionRow::result(192, 8, vec![], Verdict::Pass)),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .baseline_provenance(baseline_prov)
        .correlation_id("suite-run-42")
        .pacing(pacing_summary)
        .environment(vec![
            ("runtime".to_string(), "tokio".to_string()),
            ("region".to_string(), "us-east-1".to_string()),
        ])
        .warning(Warning::new("BASELINE_AGING", "Baseline is 49 days old"))
        .build();
        let xml = VerdictXmlWriter::write_record(&record, Some("2026-04-19T10:00:00Z"));
        assert_snapshot!(xml);
    }
}
