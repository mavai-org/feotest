//! Transparent statistics renderer.
//!
//! Formats already-computed verdict data into a human-readable box-format
//! diagnostic. The renderer is a pure function from data to formatted text —
//! it performs no statistical calculations.

use std::fmt;

use crate::model::{TerminationReason, TestIntent};
use crate::ptest::builder::ThresholdApproach;
use crate::verdict::{StatisticalAnalysis, Verdict, VerdictRecord};

/// Box width in characters (outer border inclusive).
const BOX_WIDTH: usize = 63;

/// Formats the transparent statistics report for a verdict record.
///
/// Reads already-computed values from the record and writes the
/// canonical box-format diagnostic to the provided writer.
///
/// # Errors
///
/// Returns `fmt::Error` if writing to the writer fails.
pub fn render(
    record: &VerdictRecord,
    approach: &ThresholdApproach,
    writer: &mut dyn fmt::Write,
) -> fmt::Result {
    write_top_border(writer)?;
    write_header(record, approach, writer)?;
    write_separator(writer)?;

    // Feasibility warning (conditional)
    if has_feasibility_warning(record) {
        write_feasibility_warning(record, writer)?;
        write_separator(writer)?;
    }

    // Hypotheses
    if let Some(analysis) = record.statistical_analysis() {
        write_hypotheses(analysis.threshold(), writer)?;
        write_separator(writer)?;

        // Observed data and inference
        write_observed_data(record, approach, writer)?;
        write_separator(writer)?;
    }

    // Early termination (conditional)
    let reason = record.execution().termination().reason();
    if matches!(
        reason,
        TerminationReason::FailureInevitable | TerminationReason::SuccessGuaranteed
    ) {
        write_early_termination(record, writer)?;
        write_separator(writer)?;
    }

    // Verdict
    write_verdict(record, writer)?;
    write_bottom_border(writer)?;
    Ok(())
}

/// Formats a single-line verdict summary.
///
/// Always printed to stderr after a probabilistic test completes, regardless
/// of the `transparent_stats` setting. The detailed box report is additive —
/// this line is the baseline.
///
/// # Errors
///
/// Returns `fmt::Error` if writing to the writer fails.
pub fn render_verdict_line(record: &VerdictRecord, writer: &mut dyn fmt::Write) -> fmt::Result {
    let name = record
        .identity()
        .test_name()
        .unwrap_or_else(|| record.identity().service_contract_id());

    let verdict = match record.verdict() {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::Inconclusive => "INCONCLUSIVE",
    };

    let func = record.functional();
    let total = func.successes() + func.failures();

    if let Some(analysis) = record.statistical_analysis() {
        write!(
            writer,
            "feotest: {name} \u{2014} {verdict} ({:.3} pass rate, threshold {:.3}, n={total})",
            func.pass_rate(),
            analysis.threshold(),
        )
    } else {
        write!(
            writer,
            "feotest: {name} \u{2014} {verdict} ({:.3} pass rate, n={total})",
            func.pass_rate(),
        )
    }
}

// ---------------------------------------------------------------------------
// Box drawing
// ---------------------------------------------------------------------------

fn write_top_border(w: &mut dyn fmt::Write) -> fmt::Result {
    write!(w, "║")?;
    for _ in 0..BOX_WIDTH - 2 {
        write!(w, "═")?;
    }
    writeln!(w, "║")
}

fn write_bottom_border(w: &mut dyn fmt::Write) -> fmt::Result {
    write!(w, "║")?;
    for _ in 0..BOX_WIDTH - 2 {
        write!(w, "═")?;
    }
    writeln!(w, "║")
}

fn write_separator(w: &mut dyn fmt::Write) -> fmt::Result {
    write!(w, "║")?;
    for _ in 0..BOX_WIDTH - 2 {
        write!(w, "─")?;
    }
    writeln!(w, "║")
}

fn write_line(w: &mut dyn fmt::Write, content: &str) -> fmt::Result {
    // Inner width = BOX_WIDTH - 2 (for the two ║ chars) - 2 (for padding spaces)
    let inner = BOX_WIDTH - 4;
    if content.len() <= inner {
        writeln!(w, "║ {content:<inner$} ║")
    } else {
        // Truncate long lines rather than overflow the box
        writeln!(w, "║ {content:.inner$} ║")
    }
}

fn write_blank_line(w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(w, "")
}

// ---------------------------------------------------------------------------
// Header section
// ---------------------------------------------------------------------------

fn write_header(
    record: &VerdictRecord,
    approach: &ThresholdApproach,
    w: &mut dyn fmt::Write,
) -> fmt::Result {
    write_line(w, "TRANSPARENT STATISTICS")?;
    write_blank_line(w)?;

    // Test name
    let name = record
        .identity()
        .test_name()
        .unwrap_or_else(|| record.identity().service_contract_id());
    write_line(w, &format!("Test:       {name}"))?;

    // Approach label
    let approach_label = match approach {
        ThresholdApproach::ThresholdFirst { .. } => "Threshold-first",
        ThresholdApproach::SampleSizeFirst { .. } => "Sample-size-first",
        ThresholdApproach::ConfidenceFirst { .. } => "Confidence-first",
    };
    write_line(w, &format!("Approach:   {approach_label}"))?;

    // Intent
    write_line(w, &format!("Intent:     {}", record.intent()))?;

    // Threshold origin
    if let Some(prov) = record.spec_provenance() {
        write_line(w, &format!("Origin:     {}", prov.threshold_origin()))?;

        // Contract ref (conditional)
        if let Some(cref) = prov.contract_ref() {
            write_line(w, &format!("Contract:   {cref}"))?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Feasibility warning
// ---------------------------------------------------------------------------

fn has_feasibility_warning(record: &VerdictRecord) -> bool {
    record.warnings().iter().any(|w| w.code() == "UNDERSIZED")
}

fn write_feasibility_warning(record: &VerdictRecord, w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(w, "WARNING")?;
    write_blank_line(w)?;
    for warning in record.warnings() {
        if warning.code() == "UNDERSIZED" {
            // Wrap long warning messages across multiple lines
            let msg = warning.message();
            let inner = BOX_WIDTH - 4;
            for chunk in wrap_text(msg, inner) {
                write_line(w, &chunk)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Hypotheses
// ---------------------------------------------------------------------------

fn write_hypotheses(threshold: f64, w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(w, "HYPOTHESES")?;
    write_blank_line(w)?;
    write_line(
        w,
        &format!("H0: p >= {threshold:.3}  (service meets threshold)"),
    )?;
    write_line(
        w,
        &format!("H1: p <  {threshold:.3}  (service is degraded)"),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Observed data and inference
// ---------------------------------------------------------------------------

fn write_observed_data(
    record: &VerdictRecord,
    approach: &ThresholdApproach,
    w: &mut dyn fmt::Write,
) -> fmt::Result {
    write_line(w, "OBSERVED DATA AND INFERENCE")?;
    write_blank_line(w)?;

    let func = record.functional();
    let total = func.successes() + func.failures();
    write_line(
        w,
        &format!("Successes / Total:    {} / {}", func.successes(), total,),
    )?;
    write_line(w, &format!("Observed pass rate:   {:.3}", func.pass_rate()))?;

    if let Some(analysis) = record.statistical_analysis() {
        // Threshold with approach-specific detail on the next line
        write_line(
            w,
            &format!("Threshold:            {:.3}", analysis.threshold()),
        )?;
        let detail = approach_detail(approach, analysis.confidence_level(), record);
        write_line(w, &format!("  {detail}"))?;

        // z-statistic and p-value
        if let Some(z) = analysis.test_statistic() {
            write_line(w, &format!("z-statistic:          {z:.3}"))?;
        }
        if let Some(p) = analysis.p_value() {
            write_line(w, &format!("p-value:              {p:.3}"))?;
        }

        // Wilson one-sided lower bound at the verdict's confidence level
        write_line(
            w,
            &format!(
                "Wilson lower [{:.0}%]: {:.3}",
                analysis.confidence_level() * 100.0,
                analysis.wilson_lower(),
            ),
        )?;

        // Standard error
        write_line(
            w,
            &format!("Standard error:       {:.3}", analysis.standard_error()),
        )?;
    }

    Ok(())
}

fn approach_detail(
    approach: &ThresholdApproach,
    confidence_level: f64,
    record: &VerdictRecord,
) -> String {
    match approach {
        ThresholdApproach::ThresholdFirst { .. } => {
            format!("(implied confidence: {confidence_level:.3})")
        }
        ThresholdApproach::SampleSizeFirst { confidence, .. } => {
            format!("(derived from baseline at {confidence:.3} confidence)")
        }
        ThresholdApproach::ConfidenceFirst { .. } => {
            format!(
                "(computed sample size: {})",
                record.execution().samples_planned(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Early termination
// ---------------------------------------------------------------------------

fn write_early_termination(record: &VerdictRecord, w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(w, "EARLY TERMINATION")?;
    write_blank_line(w)?;

    let termination = record.execution().termination();
    let label = match termination.reason() {
        TerminationReason::FailureInevitable => "Failure inevitable",
        TerminationReason::SuccessGuaranteed => "Success guaranteed",
        _ => "Other",
    };
    write_line(w, &format!("Reason:     {label}"))?;
    write_line(
        w,
        &format!(
            "Executed:   {} of {} planned samples",
            record.execution().samples_executed(),
            record.execution().samples_planned(),
        ),
    )?;

    if let Some(detail) = termination.detail() {
        write_line(w, &format!("Detail:     {detail}"))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Verdict
// ---------------------------------------------------------------------------

fn write_verdict(record: &VerdictRecord, w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(w, "VERDICT")?;
    write_blank_line(w)?;

    let verdict_label = match record.verdict() {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::Inconclusive => "INCONCLUSIVE",
    };

    if record.intent() == TestIntent::Smoke {
        write_line(
            w,
            &format!("{verdict_label} (non-evidential \u{2014} Smoke intent)"),
        )?;
    } else {
        write_line(w, verdict_label)?;
    }

    write_blank_line(w)?;

    // Reasoning text
    let reasoning = verdict_reasoning(record);
    let inner = BOX_WIDTH - 4;
    for line in wrap_text(&reasoning, inner) {
        write_line(w, &line)?;
    }

    Ok(())
}

fn verdict_reasoning(record: &VerdictRecord) -> String {
    let func = record.functional();
    match record.verdict() {
        Verdict::Pass => {
            format!(
                "The observed success rate of {:.3} is consistent with \
                 the baseline expectation. No evidence of degradation.",
                func.pass_rate(),
            )
        }
        Verdict::Fail => {
            let threshold = record
                .statistical_analysis()
                .map_or(0.0, StatisticalAnalysis::threshold);
            format!(
                "The observed success rate of {:.3} is significantly \
                 below the threshold of {:.3}. The null hypothesis is \
                 rejected.",
                func.pass_rate(),
                threshold,
            )
        }
        Verdict::Inconclusive => "Insufficient evidence to reach a conclusion. The \
             statistical analysis is unreliable at this sample size."
            .to_string(),
    }
}

// ---------------------------------------------------------------------------
// Text wrapping
// ---------------------------------------------------------------------------

fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line = word.to_string();
        } else if current_line.len() + 1 + word.len() <= max_width {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            lines.push(current_line);
            current_line = word.to_string();
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity,
        TestIntent, ThresholdOrigin, Warning,
    };
    use crate::verdict::{FunctionalDimension, SpecProvenance, StatisticalAnalysis};
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

    fn sample_execution_early(
        planned: u32,
        executed: u32,
        successes: u32,
        failures: u32,
        reason: TerminationReason,
    ) -> ExecutionSummary {
        ExecutionSummary::new(
            planned,
            executed,
            successes,
            failures,
            TerminationInfo::new(reason),
            CostSummary::new(Duration::from_millis(200), 500, executed),
        )
    }

    fn pass_record() -> VerdictRecord {
        let analysis =
            StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(2.294, 0.011);
        let provenance =
            SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("my-service.yaml");

        VerdictRecord::builder(
            TestIdentity::new("my-service"),
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

    // -----------------------------------------------------------------------
    // Snapshot tests
    // -----------------------------------------------------------------------

    #[test]
    fn pass_verdict_threshold_first() {
        let record = pass_record();
        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn fail_verdict_sample_size_first() {
        let record = fail_record();
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn fail_verdict_confidence_first() {
        let record = fail_record();
        let approach = ThresholdApproach::ConfidenceFirst {
            confidence: 0.95,
            min_detectable_effect: 0.05,
            power: 0.80,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn inconclusive_verdict() {
        let record = inconclusive_record();
        let approach = ThresholdApproach::ThresholdFirst {
            samples: 10,
            min_pass_rate: 0.80,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn smoke_intent_label() {
        let analysis = StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Sla)
            .with_test_results(2.294, 0.011);
        let provenance =
            SpecProvenance::new(ThresholdOrigin::Sla).with_contract_ref("API SLA v3.2 §2.1");

        let record = VerdictRecord::builder(
            TestIdentity::new("my-service"),
            Verdict::Pass,
            TestIntent::Smoke,
            sample_execution(10, 10, 10, 0),
            FunctionalDimension::new(10, 0, vec![]),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .warning(Warning::new(
            "SMOKE_NORMATIVE",
            "Smoke test against normative threshold — verdict is not evidential",
        ))
        .build();

        let approach = ThresholdApproach::ThresholdFirst {
            samples: 10,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn early_termination_failure_inevitable() {
        let record = VerdictRecord::builder(
            TestIdentity::new("degraded-service"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution_early(100, 42, 20, 22, TerminationReason::FailureInevitable),
            FunctionalDimension::new(20, 22, vec![]),
        )
        .statistical_analysis(
            StatisticalAnalysis::new(0.95, 0.077, 0.342, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(-5.195, 1.000),
        )
        .spec_provenance(SpecProvenance::new(ThresholdOrigin::Empirical))
        .build();

        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn early_termination_success_guaranteed() {
        let record = VerdictRecord::builder(
            TestIdentity::new("solid-service"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution_early(100, 60, 58, 2, TerminationReason::SuccessGuaranteed),
            FunctionalDimension::new(58, 2, vec![]),
        )
        .statistical_analysis(
            StatisticalAnalysis::new(0.95, 0.025, 0.918, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(3.867, 0.000),
        )
        .spec_provenance(SpecProvenance::new(ThresholdOrigin::Empirical))
        .build();

        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn feasibility_warning() {
        let analysis = StatisticalAnalysis::new(0.95, 0.075, 0.753, 0.950, ThresholdOrigin::Sla)
            .with_test_results(0.667, 0.252);
        let provenance = SpecProvenance::new(ThresholdOrigin::Sla);

        let record = VerdictRecord::builder(
            TestIdentity::new("tiny-test"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(10, 10, 10, 0),
            FunctionalDimension::new(10, 0, vec![]),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .warning(Warning::new(
            "UNDERSIZED",
            "Sample size 10 is insufficient for verification-grade evidence at threshold 0.9500",
        ))
        .build();

        let approach = ThresholdApproach::ThresholdFirst {
            samples: 10,
            min_pass_rate: 0.95,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn contract_ref_present() {
        let analysis = StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Sla)
            .with_test_results(2.294, 0.011);
        let provenance = SpecProvenance::new(ThresholdOrigin::Sla)
            .with_spec_filename("payment-gateway.yaml")
            .with_contract_ref("API SLA v3.2 §2.1");

        let record = VerdictRecord::builder(
            TestIdentity::new("payment-gateway"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 96, 4),
            FunctionalDimension::new(96, 4, vec![]),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .build();

        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();

        // Verify the contract line is present
        assert!(buf.contains("Contract:   API SLA v3.2 §2.1"));
    }

    #[test]
    fn contract_ref_absent() {
        let analysis =
            StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(2.294, 0.011);
        let provenance =
            SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("my-service.yaml");

        let record = VerdictRecord::builder(
            TestIdentity::new("my-service"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 96, 4),
            FunctionalDimension::new(96, 4, vec![]),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .build();

        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();

        // Contract line must not appear
        assert!(!buf.contains("Contract:"));
    }

    #[test]
    fn box_width_63() {
        let record = pass_record();
        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();

        for line in buf.lines() {
            // Each line should be exactly BOX_WIDTH characters when measured
            // in Unicode grapheme clusters. For our box-drawing characters,
            // char count is a reasonable proxy.
            let char_count: usize = line.chars().count();
            assert!(
                char_count <= BOX_WIDTH,
                "Line exceeds {BOX_WIDTH} chars ({char_count}): {line:?}"
            );
        }
    }

    #[test]
    fn three_decimal_places() {
        let record = pass_record();
        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();

        // Verify key numeric values use 3 decimal places
        assert!(buf.contains("0.960")); // pass rate
        assert!(buf.contains("0.900")); // threshold
        assert!(buf.contains("2.294")); // z-statistic
        assert!(buf.contains("0.011")); // p-value
        assert!(buf.contains("0.907")); // Wilson one-sided lower bound
        assert!(buf.contains("0.022")); // standard error
    }

    #[test]
    fn test_name_used_when_present() {
        let analysis =
            StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(2.294, 0.011);

        let record = VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_translation_accuracy"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 96, 4),
            FunctionalDimension::new(96, 4, vec![]),
        )
        .statistical_analysis(analysis)
        .build();

        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();

        assert!(buf.contains("test_translation_accuracy"));
        assert!(!buf.contains("Test:       my-service"));
    }

    #[test]
    fn sample_size_first_approach_detail() {
        let record = pass_record();
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();

        assert!(buf.contains("Sample-size-first"));
        assert!(buf.contains("derived from baseline at 0.950 confidence"));
    }

    #[test]
    fn confidence_first_approach_detail() {
        let record = pass_record();
        let approach = ThresholdApproach::ConfidenceFirst {
            confidence: 0.95,
            min_detectable_effect: 0.05,
            power: 0.80,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();

        assert!(buf.contains("Confidence-first"));
        assert!(buf.contains("computed sample size: 100"));
    }

    #[test]
    fn threshold_first_approach_detail() {
        let record = pass_record();
        let approach = ThresholdApproach::ThresholdFirst {
            samples: 100,
            min_pass_rate: 0.90,
        };
        let mut buf = String::new();
        render(&record, &approach, &mut buf).unwrap();

        assert!(buf.contains("Threshold-first"));
        assert!(buf.contains("implied confidence: 0.950"));
    }

    // -----------------------------------------------------------------------
    // Verdict line tests
    // -----------------------------------------------------------------------

    #[test]
    fn verdict_line_pass_with_stats() {
        let record = pass_record();
        let mut line = String::new();
        render_verdict_line(&record, &mut line).unwrap();

        assert_eq!(
            line,
            "feotest: my-service \u{2014} PASS (0.960 pass rate, threshold 0.900, n=100)"
        );
    }

    #[test]
    fn verdict_line_fail_with_stats() {
        let record = fail_record();
        let mut line = String::new();
        render_verdict_line(&record, &mut line).unwrap();

        assert_eq!(
            line,
            "feotest: my-service \u{2014} FAIL (0.800 pass rate, threshold 0.900, n=100)"
        );
    }

    #[test]
    fn verdict_line_inconclusive_without_stats() {
        let record = inconclusive_record();
        let mut line = String::new();
        render_verdict_line(&record, &mut line).unwrap();

        assert_eq!(
            line,
            "feotest: flaky-service \u{2014} INCONCLUSIVE (0.700 pass rate, n=10)"
        );
    }

    #[test]
    fn verdict_line_uses_test_name_when_present() {
        let analysis =
            StatisticalAnalysis::new(0.95, 0.022, 0.907, 0.900, ThresholdOrigin::Empirical)
                .with_test_results(2.294, 0.011);

        let record = VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_translation"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(100, 100, 96, 4),
            FunctionalDimension::new(96, 4, vec![]),
        )
        .statistical_analysis(analysis)
        .build();

        let mut line = String::new();
        render_verdict_line(&record, &mut line).unwrap();

        assert!(line.contains("test_translation"));
        assert!(!line.contains("my-service"));
    }
}
