//! Transparent statistics renderer.
//!
//! Formats already-computed verdict data into a human-readable box-format
//! diagnostic. The renderer is a pure function from data to formatted text —
//! it performs no statistical calculations: every figure, including each
//! decision rule's evidence, is read from the record.

// mavai-ref: JVI-G3NPRSS — do not remove (resolves in mavai-orchestrator)

use std::fmt;

use crate::model::{TerminationReason, TestIntent};
use crate::ptest::builder::ThresholdApproach;
use crate::verdict::{
    ComplianceEvidence, RegressionEvidence, RuleEvidence, StatisticalAnalysis, Verdict,
    VerdictRecord,
};

/// Box width in characters (outer border inclusive).
const BOX_WIDTH: usize = 63;

/// The environment key under which a threshold-first plan discloses the
/// implied alpha of its cutoff.
const IMPLIED_ALPHA_KEY: &str = "sizing-implied-alpha";

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

    if record.is_refused() {
        write_refusal(record, writer)?;
        write_bottom_border(writer)?;
        return Ok(());
    }

    // Feasibility warning (conditional)
    if has_feasibility_warning(record) {
        write_feasibility_warning(record, writer)?;
        write_separator(writer)?;
    }

    // Hypotheses, observed data and the rule's evidence
    if let Some(analysis) = record.statistical_analysis() {
        write_hypotheses(analysis, writer)?;
        write_separator(writer)?;

        write_observed_data(record, analysis, approach, writer)?;
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

/// The verdict's label, `REFUSED` for a refused configuration.
const fn verdict_label(record: &VerdictRecord) -> &'static str {
    match record.verdict() {
        Some(Verdict::Pass) => "PASS",
        Some(Verdict::Fail) => "FAIL",
        Some(Verdict::Inconclusive) => "INCONCLUSIVE",
        None => "REFUSED",
    }
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
    let verdict = verdict_label(record);

    let Some(func) = record.functional_summary() else {
        return write!(
            writer,
            "feotest: {name} \u{2014} {verdict} ({})",
            record.verdict_reason()
        );
    };
    if let Some(analysis) = record.statistical_analysis() {
        write!(
            writer,
            "feotest: {name} \u{2014} {verdict} ({:.3} pass rate, threshold {:.3}, {}, n={})",
            func.pass_rate(),
            analysis.threshold(),
            analysis.decision_rule(),
            func.total(),
        )
    } else {
        write!(
            writer,
            "feotest: {name} \u{2014} {verdict} ({:.3} pass rate, n={})",
            func.pass_rate(),
            func.total(),
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
    if content.chars().count() <= inner {
        let padding = inner - content.chars().count();
        writeln!(w, "║ {content}{:padding$} ║", "")
    } else {
        // Truncate long lines rather than overflow the box
        let truncated: String = content.chars().take(inner).collect();
        writeln!(w, "║ {truncated} ║")
    }
}

fn write_blank_line(w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(w, "")
}

fn write_wrapped(w: &mut dyn fmt::Write, text: &str) -> fmt::Result {
    for line in wrap_text(text, BOX_WIDTH - 4) {
        write_line(w, &line)?;
    }
    Ok(())
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

    let name = record
        .identity()
        .test_name()
        .unwrap_or_else(|| record.identity().service_contract_id());
    write_line(w, &format!("Test:        {name}"))?;
    write_line(w, &format!("Approach:    {}", approach.canonical_name()))?;
    write_line(w, &format!("Intent:      {}", record.intent()))?;
    write_line(w, &format!("Methodology: {}", record.methodology_version()))?;

    if let Some(prov) = record.spec_provenance() {
        write_line(w, &format!("Origin:      {}", prov.threshold_origin()))?;
        if let Some(cref) = prov.contract_ref() {
            write_line(w, &format!("Contract:    {cref}"))?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Refusal
// ---------------------------------------------------------------------------

fn write_refusal(record: &VerdictRecord, w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(w, "CONFIGURATION REFUSED")?;
    write_blank_line(w)?;
    for error in record.configuration_errors() {
        write_line(w, error.code())?;
    }
    if let Some(detail) = record.execution().termination().detail() {
        write_blank_line(w)?;
        write_wrapped(w, detail)?;
    }
    write_blank_line(w)?;
    write_wrapped(
        w,
        "No sample ran and there is no verdict: the configuration was refused whole \
         before the run.",
    )
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
            write_wrapped(w, warning.message())?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Hypotheses
// ---------------------------------------------------------------------------

fn write_hypotheses(analysis: &StatisticalAnalysis, w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(w, "HYPOTHESES")?;
    write_blank_line(w)?;
    match analysis.evidence() {
        RuleEvidence::Compliance(evidence) => {
            let requirement = evidence.requirement;
            write_line(
                w,
                &format!("H0: p <= {requirement:.3}  (compliance not demonstrated)"),
            )?;
            write_line(
                w,
                &format!("H1: p >  {requirement:.3}  (the requirement is met)"),
            )
        }
        RuleEvidence::Regression(_) => {
            write_line(w, "H0: p_test = p_baseline  (no degradation)")?;
            write_line(w, "H1: p_test < p_baseline  (the service has degraded)")
        }
    }
}

// ---------------------------------------------------------------------------
// Observed data and inference
// ---------------------------------------------------------------------------

fn write_observed_data(
    record: &VerdictRecord,
    analysis: &StatisticalAnalysis,
    approach: &ThresholdApproach,
    w: &mut dyn fmt::Write,
) -> fmt::Result {
    write_line(w, "OBSERVED DATA AND INFERENCE")?;
    write_blank_line(w)?;

    if let Some(func) = record.functional_summary() {
        write_line(
            w,
            &format!("Successes / Total:    {} / {}", func.pass(), func.total()),
        )?;
        write_line(w, &format!("Observed pass rate:   {:.3}", func.pass_rate()))?;
    }
    write_line(
        w,
        &format!(
            "Decision rule:        {} v{}",
            analysis.decision_rule(),
            analysis.decision_rule().version()
        ),
    )?;
    write_line(
        w,
        &format!("Threshold:            {:.3}", analysis.threshold()),
    )?;
    let detail = approach_detail(approach, record);
    write_line(w, &format!("  {detail}"))?;

    match analysis.evidence() {
        RuleEvidence::Compliance(evidence) => write_compliance_evidence(evidence, w)?,
        RuleEvidence::Regression(evidence) => write_regression_evidence(evidence, w)?,
    }

    write_line(
        w,
        &format!(
            "Wilson lower [{:.0}%]: {:.3} (descriptive)",
            analysis.confidence_level() * 100.0,
            analysis.wilson_lower(),
        ),
    )?;
    write_line(
        w,
        &format!("Standard error:       {:.3}", analysis.standard_error()),
    )
}

fn write_compliance_evidence(evidence: &ComplianceEvidence, w: &mut dyn fmt::Write) -> fmt::Result {
    match evidence.minimum_passing_count {
        Some(k_min) => write_line(w, &format!("Smallest passing:     {k_min}"))?,
        None => write_line(w, "Smallest passing:     none (no count can pass)")?,
    }
    write_line(
        w,
        &format!("False compliance:     {:.4}", evidence.false_compliance),
    )?;
    write_line(
        w,
        &format!(
            "Clopper-Pearson lower: {:.3} (descriptive)",
            evidence.clopper_pearson_lower
        ),
    )
}

fn write_regression_evidence(evidence: &RegressionEvidence, w: &mut dyn fmt::Write) -> fmt::Result {
    write_line(
        w,
        &format!(
            "Baseline:             {} / {}",
            evidence.baseline_successes, evidence.baseline_trials
        ),
    )?;
    write_line(w, &format!("Cutoff:               {}", evidence.cutoff))?;
    if let Some(size) = evidence.size_at_assumed_common_rate {
        write_line(w, &format!("Size at common rate:  {size:.4}"))?;
    }
    match evidence.minimum_detectable_degradation {
        Some(drop) => write_line(
            w,
            &format!("Min detectable drop:  {drop:.4} (80% design power)"),
        )?,
        None => write_line(w, "Min detectable drop:  none at 80% design power")?,
    }
    if let Some(rate) = evidence.design_alternative_rate {
        write_line(w, &format!("Design alternative:   {rate:.4}"))?;
    }
    if let Some(power) = evidence.design_power {
        write_line(w, &format!("Design power:         {power:.4}"))?;
    }
    if let Some(power) = evidence.resolved_test_power {
        write_line(w, &format!("Resolved power:       {power:.4}"))?;
    }
    Ok(())
}

fn approach_detail(approach: &ThresholdApproach, record: &VerdictRecord) -> String {
    match approach {
        ThresholdApproach::ThresholdFirst { .. } => record
            .environment()
            .iter()
            .find(|(key, _)| key == IMPLIED_ALPHA_KEY)
            .map_or_else(
                || "(declared floor; each criterion decided by its rule)".to_owned(),
                |(_, alpha)| format!("(implied alpha of the declared cutoff: {alpha})"),
            ),
        ThresholdApproach::SampleSizeFirst { confidence, .. } => {
            format!("(Fisher cutoff at {confidence:.3} confidence)")
        }
        ThresholdApproach::ConfidenceFirst {
            min_detectable_effect,
            ..
        } => {
            format!(
                "(n={} sized to detect a drop of {min_detectable_effect:.3})",
                record.execution().samples_planned(),
            )
        }
        ThresholdApproach::RiskDriven {
            design_alternative_rate,
            ..
        } => {
            format!(
                "(n={} sized for a true rate of {design_alternative_rate:.3})",
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

    let label = verdict_label(record);
    if record.intent() == TestIntent::Smoke {
        write_line(
            w,
            &format!("{label} (non-evidential \u{2014} Smoke intent)"),
        )?;
    } else {
        write_line(w, label)?;
    }

    write_blank_line(w)?;
    write_wrapped(w, &verdict_reasoning(record))
}

fn verdict_reasoning(record: &VerdictRecord) -> String {
    let rule = record
        .statistical_analysis()
        .map_or_else(|| "its rule".to_owned(), |a| a.decision_rule().to_string());
    let functional_binds = record.functional_assessment().mode().is_enforced();
    let latency_advisory = record
        .latency()
        .is_some_and(|dimension| !dimension.mode().is_enforced());
    match record.verdict() {
        Some(Verdict::Pass) if functional_binds && !latency_advisory => format!(
            "Every criterion passed under its rule ({rule} for the first) and every \
             enforced latency constraint passed: {}.",
            record.verdict_reason()
        ),
        Some(Verdict::Pass) => format!(
            "Every enforced assertion passed; an advisory dimension is decided and \
             reported, and never fails the test: {}.",
            record.verdict_reason()
        ),
        Some(Verdict::Fail) => format!("The test failed: {}.", record.verdict_reason()),
        Some(Verdict::Inconclusive) => format!(
            "The data cannot decide the test: {}.",
            record.verdict_reason()
        ),
        None => record.verdict_reason().to_owned(),
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
    use crate::oracle_examples::{analysis_of, compliance_row, regression_row};
    use crate::statistics::rules::ConfigurationError;
    use crate::verdict::{CriterionRow, FunctionalAssessment, SpecProvenance};
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // Test helpers — every figure comes from a named oracle fixture case
    // -----------------------------------------------------------------------

    fn execution(planned: u32, row: &CriterionRow, reason: TerminationReason) -> ExecutionSummary {
        ExecutionSummary::new(
            planned,
            row.total(),
            row.pass(),
            row.fail(),
            TerminationInfo::new(reason),
            CostSummary::new(Duration::from_millis(500), 1000, row.total()),
        )
    }

    /// A record decided by one oracle row.
    fn record_of(
        name: &str,
        row: CriterionRow,
        planned: u32,
        reason: TerminationReason,
        intent: TestIntent,
        provenance: SpecProvenance,
    ) -> VerdictRecord {
        let analysis = analysis_of(&row);
        VerdictRecord::builder(
            TestIdentity::new(name),
            row.verdict(),
            intent,
            execution(planned, &row, reason),
            FunctionalAssessment::single(row),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .build()
    }

    /// Oracle case `worked_example_pass_above_cutoff`: 97 of 100, cutoff 91.
    fn pass_record() -> VerdictRecord {
        record_of(
            "my-service",
            regression_row("worked_example_pass_above_cutoff", "result"),
            100,
            TerminationReason::Completed,
            TestIntent::Verification,
            SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("my-service.yaml"),
        )
    }

    /// Oracle case `worked_example_fail_deep_degradation`: 80 of 100, cutoff 91.
    fn fail_record() -> VerdictRecord {
        record_of(
            "my-service",
            regression_row("worked_example_fail_deep_degradation", "result"),
            100,
            TerminationReason::Completed,
            TestIntent::Verification,
            SpecProvenance::new(ThresholdOrigin::Empirical).with_spec_filename("my-service.yaml"),
        )
    }

    /// Oracle case `p95_n150_pass_at_k_min`: 148 of 150 demonstrate 0.95.
    fn compliance_record(intent: TestIntent) -> VerdictRecord {
        record_of(
            "payment-gateway",
            compliance_row("p95_n150_pass_at_k_min", "result", ThresholdOrigin::Sla),
            150,
            TerminationReason::Completed,
            intent,
            SpecProvenance::new(ThresholdOrigin::Sla).with_contract_ref("API SLA v3.2 §2.1"),
        )
    }

    fn inconclusive_record() -> VerdictRecord {
        VerdictRecord::builder(
            TestIdentity::new("flaky-service"),
            Verdict::Inconclusive,
            TestIntent::Verification,
            ExecutionSummary::new(
                10,
                10,
                7,
                3,
                TerminationInfo::new(TerminationReason::Completed),
                CostSummary::new(Duration::from_millis(500), 1000, 10),
            ),
            FunctionalAssessment::single(CriterionRow::result(7, 3, vec![], Verdict::Inconclusive)),
        )
        .build()
    }

    fn render_with(record: &VerdictRecord, approach: &ThresholdApproach) -> String {
        let mut buf = String::new();
        render(record, approach, &mut buf).unwrap();
        buf
    }

    fn threshold_first(samples: u32, min_pass_rate: f64) -> ThresholdApproach {
        ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate,
        }
    }

    // -----------------------------------------------------------------------
    // Snapshot tests
    // -----------------------------------------------------------------------

    #[test]
    fn pass_verdict_threshold_first() {
        insta::assert_snapshot!(render_with(&pass_record(), &threshold_first(100, 0.90)));
    }

    #[test]
    fn fail_verdict_sample_size_first() {
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        insta::assert_snapshot!(render_with(&fail_record(), &approach));
    }

    #[test]
    fn fail_verdict_confidence_first() {
        let approach = ThresholdApproach::ConfidenceFirst {
            confidence: 0.95,
            min_detectable_effect: 0.05,
            power: 0.80,
        };
        insta::assert_snapshot!(render_with(&fail_record(), &approach));
    }

    #[test]
    fn inconclusive_verdict() {
        insta::assert_snapshot!(render_with(
            &inconclusive_record(),
            &threshold_first(10, 0.80)
        ));
    }

    #[test]
    fn compliance_verdict() {
        insta::assert_snapshot!(render_with(
            &compliance_record(TestIntent::Verification),
            &threshold_first(150, 0.0)
        ));
    }

    #[test]
    fn smoke_intent_label() {
        let record = compliance_record(TestIntent::Smoke);
        let buf = render_with(&record, &threshold_first(150, 0.0));
        insta::assert_snapshot!(buf);
    }

    /// Oracle case `small_test_fail_below_cutoff`, stopped early: 17 of 25
    /// executed of 100 planned.
    #[test]
    fn early_termination_failure_inevitable() {
        let record = record_of(
            "degraded-service",
            regression_row("small_test_fail_below_cutoff", "result"),
            100,
            TerminationReason::FailureInevitable,
            TestIntent::Verification,
            SpecProvenance::new(ThresholdOrigin::Empirical),
        );
        insta::assert_snapshot!(render_with(&record, &threshold_first(100, 0.90)));
    }

    /// Oracle case `near_perfect_baseline_pass_at_cutoff`, stopped early on
    /// success: 94 of 100 executed of 120 planned.
    #[test]
    fn early_termination_success_guaranteed() {
        let record = record_of(
            "solid-service",
            regression_row("near_perfect_baseline_pass_at_cutoff", "result"),
            120,
            TerminationReason::SuccessGuaranteed,
            TestIntent::Verification,
            SpecProvenance::new(ThresholdOrigin::Empirical),
        );
        insta::assert_snapshot!(render_with(&record, &threshold_first(120, 0.90)));
    }

    /// Oracle case `headline_995_n477_smoke_pass_not_possible`: 477 of 477
    /// cannot demonstrate 0.995 at alpha 0.05, so a smoke run fails.
    #[test]
    fn feasibility_warning() {
        let row = compliance_row(
            "headline_995_n477_smoke_pass_not_possible",
            "result",
            ThresholdOrigin::Sla,
        );
        let feasibility = crate::statistics::feasibility::feasibility_check(477, 0.995, 0.05);
        let analysis = analysis_of(&row);
        let record = VerdictRecord::builder(
            TestIdentity::new("tiny-test"),
            Verdict::Fail,
            TestIntent::Smoke,
            execution(477, &row, TerminationReason::Completed),
            FunctionalAssessment::single(row),
        )
        .statistical_analysis(analysis)
        .spec_provenance(SpecProvenance::new(ThresholdOrigin::Sla))
        .warning(Warning::new(
            "UNDERSIZED",
            format!(
                "no count of 477 can demonstrate 0.995 at alpha 0.05 (feasibility minimum {})",
                feasibility.minimum_samples()
            ),
        ))
        .build();
        insta::assert_snapshot!(render_with(&record, &threshold_first(477, 0.0)));
    }

    #[test]
    fn refused_configuration() {
        let record = VerdictRecord::refused(
            TestIdentity::new("offer-extraction"),
            TestIntent::Verification,
            ExecutionSummary::new(
                200,
                0,
                0,
                0,
                TerminationInfo::new(TerminationReason::ConfigurationRefused).with_detail(
                    "regression: the test (200 samples) is larger than its baseline (100 \
                     trials); compliance: no count of 200 can demonstrate 0.999 at alpha 0.05 \
                     (feasibility minimum 2995)",
                ),
                CostSummary::new(Duration::ZERO, 0, 0),
            ),
            vec![
                ConfigurationError::TestLargerThanBaseline,
                ConfigurationError::ComplianceInfeasible,
            ],
        )
        .build();
        let buf = render_with(&record, &threshold_first(200, 0.0));
        assert!(buf.contains("CONFIGURATION REFUSED"));
        assert!(!buf.contains("HYPOTHESES"));
        insta::assert_snapshot!(buf);
    }

    #[test]
    fn contract_ref_present() {
        let buf = render_with(
            &compliance_record(TestIntent::Verification),
            &threshold_first(150, 0.0),
        );
        assert!(buf.contains("Contract:    API SLA v3.2 §2.1"));
        assert!(buf.contains("H0: p <= 0.950"));
    }

    #[test]
    fn contract_ref_absent() {
        let buf = render_with(&pass_record(), &threshold_first(100, 0.90));
        assert!(!buf.contains("Contract:"));
    }

    #[test]
    fn box_width_63() {
        let buf = render_with(&pass_record(), &threshold_first(100, 0.90));
        for line in buf.lines() {
            let char_count: usize = line.chars().count();
            assert!(
                char_count <= BOX_WIDTH,
                "Line exceeds {BOX_WIDTH} chars ({char_count}): {line:?}"
            );
        }
    }

    #[test]
    fn the_rule_and_its_evidence_are_shown() {
        let buf = render_with(&pass_record(), &threshold_first(100, 0.90));
        assert!(buf.contains("0.970")); // observed pass rate
        assert!(buf.contains("Decision rule:        regression/fisher v1"));
        assert!(buf.contains("Cutoff:               91"));
        assert!(buf.contains("Baseline:             951 / 1000"));
        assert!(buf.contains("(descriptive)"));
    }

    #[test]
    fn test_name_used_when_present() {
        let row = regression_row("worked_example_pass_above_cutoff", "result");
        let analysis = analysis_of(&row);
        let record = VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_translation_accuracy"),
            Verdict::Pass,
            TestIntent::Verification,
            execution(100, &row, TerminationReason::Completed),
            FunctionalAssessment::single(row),
        )
        .statistical_analysis(analysis)
        .build();
        let buf = render_with(&record, &threshold_first(100, 0.90));
        assert!(buf.contains("test_translation_accuracy"));
        assert!(!buf.contains("Test:        my-service"));
    }

    #[test]
    fn sample_size_first_approach_detail() {
        let approach = ThresholdApproach::SampleSizeFirst {
            samples: 100,
            confidence: 0.95,
        };
        let buf = render_with(&pass_record(), &approach);
        assert!(buf.contains("sample-size-first"));
        assert!(buf.contains("Fisher cutoff at 0.950 confidence"));
    }

    #[test]
    fn confidence_first_approach_detail() {
        let approach = ThresholdApproach::ConfidenceFirst {
            confidence: 0.95,
            min_detectable_effect: 0.05,
            power: 0.80,
        };
        let buf = render_with(&pass_record(), &approach);
        assert!(buf.contains("Approach:    confidence-first"));
        assert!(buf.contains("n=100 sized to detect a drop of 0.050"));
    }

    #[test]
    fn pass_verdict_risk_driven() {
        let approach = ThresholdApproach::RiskDriven {
            design_alternative_rate: 0.85,
            confidence: 0.95,
            target_power: 0.80,
        };
        insta::assert_snapshot!(render_with(&pass_record(), &approach));
    }

    #[test]
    fn risk_driven_approach_detail() {
        let approach = ThresholdApproach::RiskDriven {
            design_alternative_rate: 0.85,
            confidence: 0.95,
            target_power: 0.80,
        };
        let buf = render_with(&pass_record(), &approach);
        assert!(buf.contains("confidence-first (risk-driven)"));
        assert!(buf.contains("n=100 sized for a true rate of 0.850"));
    }

    #[test]
    fn threshold_first_approach_detail() {
        let buf = render_with(&pass_record(), &threshold_first(100, 0.90));
        assert!(buf.contains("threshold-first"));
        assert!(buf.contains("declared floor"));
    }

    // -----------------------------------------------------------------------
    // Verdict line tests
    // -----------------------------------------------------------------------

    #[test]
    fn verdict_line_pass_with_stats() {
        let mut line = String::new();
        render_verdict_line(&pass_record(), &mut line).unwrap();
        assert_eq!(
            line,
            "feotest: my-service \u{2014} PASS (0.970 pass rate, threshold 0.910, \
             regression/fisher, n=100)"
        );
    }

    #[test]
    fn verdict_line_fail_with_stats() {
        let mut line = String::new();
        render_verdict_line(&fail_record(), &mut line).unwrap();
        assert_eq!(
            line,
            "feotest: my-service \u{2014} FAIL (0.800 pass rate, threshold 0.910, \
             regression/fisher, n=100)"
        );
    }

    #[test]
    fn verdict_line_inconclusive_without_stats() {
        let mut line = String::new();
        render_verdict_line(&inconclusive_record(), &mut line).unwrap();
        assert_eq!(
            line,
            "feotest: flaky-service \u{2014} INCONCLUSIVE (0.700 pass rate, n=10)"
        );
    }

    #[test]
    fn verdict_line_uses_test_name_when_present() {
        let row = regression_row("worked_example_pass_above_cutoff", "result");
        let analysis = analysis_of(&row);
        let record = VerdictRecord::builder(
            TestIdentity::new("my-service").with_test_name("test_translation"),
            Verdict::Pass,
            TestIntent::Verification,
            execution(100, &row, TerminationReason::Completed),
            FunctionalAssessment::single(row),
        )
        .statistical_analysis(analysis)
        .build();
        let mut line = String::new();
        render_verdict_line(&record, &mut line).unwrap();
        assert!(line.contains("test_translation"));
        assert!(!line.contains("my-service"));
    }

    #[test]
    fn an_advisory_functional_dimension_is_not_claimed_to_have_passed() {
        let row = regression_row("worked_example_fail_deep_degradation", "result");
        let record = VerdictRecord::builder(
            TestIdentity::new("my-service"),
            Verdict::Pass,
            TestIntent::Verification,
            execution(100, &row, TerminationReason::Completed),
            FunctionalAssessment::single(row)
                .with_mode(crate::statistics::rules::EnforcementMode::Advisory),
        )
        .build();
        let reasoning = verdict_reasoning(&record);
        assert!(reasoning.starts_with("Every enforced assertion passed; an advisory dimension"));
        assert!(reasoning.ends_with("no assertion enforced; functional advisory."));
    }
}
