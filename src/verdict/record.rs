//! The verdict record: the single source of truth for all verdict rendering.

use crate::model::{ExecutionSummary, TestIdentity, TestIntent, ThresholdOrigin, Warning};
use crate::verdict::Verdict;

/// The complete record of a probabilistic test verdict.
///
/// This is consumed by all rendering paths: `JUnit` XML, HTML reports,
/// console output, and sentinel verdict sinks.
#[derive(Debug, Clone)]
pub struct VerdictRecord {
    identity: TestIdentity,
    verdict: Verdict,
    intent: TestIntent,
    execution: ExecutionSummary,
    functional: FunctionalDimension,
    statistical_analysis: Option<StatisticalAnalysis>,
    spec_provenance: Option<SpecProvenance>,
    warnings: Vec<Warning>,
}

impl VerdictRecord {
    /// Starts building a new verdict record.
    #[must_use]
    pub const fn builder(
        identity: TestIdentity,
        verdict: Verdict,
        intent: TestIntent,
        execution: ExecutionSummary,
        functional: FunctionalDimension,
    ) -> VerdictRecordBuilder {
        VerdictRecordBuilder {
            identity,
            verdict,
            intent,
            execution,
            functional,
            statistical_analysis: None,
            spec_provenance: None,
            warnings: Vec::new(),
        }
    }

    /// The test/experiment identity.
    #[must_use]
    pub const fn identity(&self) -> &TestIdentity {
        &self.identity
    }

    /// The overall verdict.
    #[must_use]
    pub const fn verdict(&self) -> Verdict {
        self.verdict
    }

    /// The declared test intent.
    #[must_use]
    pub const fn intent(&self) -> TestIntent {
        self.intent
    }

    /// Execution summary (samples, timing, termination).
    #[must_use]
    pub const fn execution(&self) -> &ExecutionSummary {
        &self.execution
    }

    /// Functional dimension (successes, failures, pass rate).
    #[must_use]
    pub const fn functional(&self) -> &FunctionalDimension {
        &self.functional
    }

    /// Statistical analysis, if performed.
    #[must_use]
    pub const fn statistical_analysis(&self) -> Option<&StatisticalAnalysis> {
        self.statistical_analysis.as_ref()
    }

    /// Baseline provenance, if a spec was used.
    #[must_use]
    pub const fn spec_provenance(&self) -> Option<&SpecProvenance> {
        self.spec_provenance.as_ref()
    }

    /// Warnings attached to this verdict.
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }
}

/// Builder for [`VerdictRecord`].
pub struct VerdictRecordBuilder {
    identity: TestIdentity,
    verdict: Verdict,
    intent: TestIntent,
    execution: ExecutionSummary,
    functional: FunctionalDimension,
    statistical_analysis: Option<StatisticalAnalysis>,
    spec_provenance: Option<SpecProvenance>,
    warnings: Vec<Warning>,
}

impl VerdictRecordBuilder {
    /// Attaches statistical analysis to the verdict.
    #[must_use]
    pub const fn statistical_analysis(mut self, analysis: StatisticalAnalysis) -> Self {
        self.statistical_analysis = Some(analysis);
        self
    }

    /// Attaches spec provenance to the verdict.
    #[must_use]
    pub fn spec_provenance(mut self, provenance: SpecProvenance) -> Self {
        self.spec_provenance = Some(provenance);
        self
    }

    /// Adds a warning to the verdict.
    #[must_use]
    pub fn warning(mut self, warning: Warning) -> Self {
        self.warnings.push(warning);
        self
    }

    /// Builds the verdict record.
    #[must_use]
    pub fn build(self) -> VerdictRecord {
        VerdictRecord {
            identity: self.identity,
            verdict: self.verdict,
            intent: self.intent,
            execution: self.execution,
            functional: self.functional,
            statistical_analysis: self.statistical_analysis,
            spec_provenance: self.spec_provenance,
            warnings: self.warnings,
        }
    }
}

/// Functional dimension of a verdict: success/failure counts and pass rate.
#[derive(Debug, Clone)]
pub struct FunctionalDimension {
    successes: u32,
    failures: u32,
    pass_rate: f64,
    failure_distribution: Vec<(String, u32)>,
    conformance_mismatches: u32,
    example_mismatches: Vec<String>,
}

impl FunctionalDimension {
    /// Creates a new functional dimension.
    #[must_use]
    pub fn new(successes: u32, failures: u32, failure_distribution: Vec<(String, u32)>) -> Self {
        let total = successes + failures;
        let pass_rate = if total == 0 {
            0.0
        } else {
            f64::from(successes) / f64::from(total)
        };
        Self {
            successes,
            failures,
            pass_rate,
            failure_distribution,
            conformance_mismatches: 0,
            example_mismatches: Vec::new(),
        }
    }

    /// Creates a functional dimension with conformance data.
    #[must_use]
    pub fn conformance(
        mut self,
        conformance_mismatches: u32,
        example_mismatches: Vec<String>,
    ) -> Self {
        self.conformance_mismatches = conformance_mismatches;
        self.example_mismatches = example_mismatches;
        self
    }

    /// Number of successful trials.
    #[must_use]
    pub const fn successes(&self) -> u32 {
        self.successes
    }

    /// Number of failed trials.
    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// Observed pass rate.
    #[must_use]
    pub const fn pass_rate(&self) -> f64 {
        self.pass_rate
    }

    /// Distribution of failures by postcondition check name.
    #[must_use]
    pub fn failure_distribution(&self) -> &[(String, u32)] {
        &self.failure_distribution
    }

    /// Number of instance conformance mismatches.
    #[must_use]
    pub const fn conformance_mismatches(&self) -> u32 {
        self.conformance_mismatches
    }

    /// Example mismatch diffs for diagnostic reporting.
    #[must_use]
    pub fn example_mismatches(&self) -> &[String] {
        &self.example_mismatches
    }
}

/// Statistical analysis attached to a verdict.
#[derive(Debug, Clone)]
pub struct StatisticalAnalysis {
    confidence_level: f64,
    standard_error: f64,
    ci_lower: f64,
    ci_upper: f64,
    threshold: f64,
    threshold_origin: ThresholdOrigin,
    test_statistic: Option<f64>,
    p_value: Option<f64>,
}

impl StatisticalAnalysis {
    /// Creates a new statistical analysis.
    #[must_use]
    pub const fn new(
        confidence_level: f64,
        standard_error: f64,
        ci_lower: f64,
        ci_upper: f64,
        threshold: f64,
        threshold_origin: ThresholdOrigin,
    ) -> Self {
        Self {
            confidence_level,
            standard_error,
            ci_lower,
            ci_upper,
            threshold,
            threshold_origin,
            test_statistic: None,
            p_value: None,
        }
    }

    /// Attaches hypothesis test results.
    #[must_use]
    pub const fn with_test_results(mut self, test_statistic: f64, p_value: f64) -> Self {
        self.test_statistic = Some(test_statistic);
        self.p_value = Some(p_value);
        self
    }

    /// The confidence level used.
    #[must_use]
    pub const fn confidence_level(&self) -> f64 {
        self.confidence_level
    }

    /// Standard error of the observed proportion.
    #[must_use]
    pub const fn standard_error(&self) -> f64 {
        self.standard_error
    }

    /// Lower bound of the confidence interval.
    #[must_use]
    pub const fn ci_lower(&self) -> f64 {
        self.ci_lower
    }

    /// Upper bound of the confidence interval.
    #[must_use]
    pub const fn ci_upper(&self) -> f64 {
        self.ci_upper
    }

    /// The threshold used for the verdict.
    #[must_use]
    pub const fn threshold(&self) -> f64 {
        self.threshold
    }

    /// Where the threshold came from.
    #[must_use]
    pub const fn threshold_origin(&self) -> ThresholdOrigin {
        self.threshold_origin
    }

    /// The z-test statistic, if computed.
    #[must_use]
    pub const fn test_statistic(&self) -> Option<f64> {
        self.test_statistic
    }

    /// The p-value, if computed.
    #[must_use]
    pub const fn p_value(&self) -> Option<f64> {
        self.p_value
    }
}

/// Provenance of the baseline spec used for threshold derivation.
#[derive(Debug, Clone)]
pub struct SpecProvenance {
    spec_filename: Option<String>,
    threshold_origin: ThresholdOrigin,
    contract_ref: Option<String>,
}

impl SpecProvenance {
    /// Creates spec provenance.
    #[must_use]
    pub const fn new(threshold_origin: ThresholdOrigin) -> Self {
        Self {
            spec_filename: None,
            threshold_origin,
            contract_ref: None,
        }
    }

    /// Sets the spec filename.
    #[must_use]
    pub fn with_spec_filename(mut self, filename: impl Into<String>) -> Self {
        self.spec_filename = Some(filename.into());
        self
    }

    /// Sets a human-readable contract reference.
    #[must_use]
    pub fn with_contract_ref(mut self, contract_ref: impl Into<String>) -> Self {
        self.contract_ref = Some(contract_ref.into());
        self
    }

    /// The spec filename, if from a file.
    #[must_use]
    pub fn spec_filename(&self) -> Option<&str> {
        self.spec_filename.as_deref()
    }

    /// The threshold origin.
    #[must_use]
    pub const fn threshold_origin(&self) -> ThresholdOrigin {
        self.threshold_origin
    }

    /// A human-readable contract reference (e.g., "API SLA v3.2 S2.1").
    #[must_use]
    pub fn contract_ref(&self) -> Option<&str> {
        self.contract_ref.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CostSummary, TerminationInfo, TerminationReason};
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

    #[test]
    fn builds_minimal_verdict_record() {
        let record = VerdictRecord::builder(
            TestIdentity::new("shopping-basket"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .build();

        assert_eq!(record.verdict(), Verdict::Pass);
        assert_eq!(record.intent(), TestIntent::Verification);
        assert_eq!(record.identity().use_case_id(), "shopping-basket");
        assert!(record.statistical_analysis().is_none());
        assert!(record.spec_provenance().is_none());
        assert!(record.warnings().is_empty());
    }

    #[test]
    fn builds_full_verdict_record() {
        let analysis = StatisticalAnalysis::new(
            0.95,
            0.0218,
            0.9073,
            0.9927,
            0.90,
            ThresholdOrigin::Empirical,
        )
        .with_test_results(2.29, 0.011);

        let provenance = SpecProvenance::new(ThresholdOrigin::Empirical)
            .with_spec_filename("shopping-basket.yaml")
            .with_contract_ref("Baseline v1");

        let record = VerdictRecord::builder(
            TestIdentity::new("shopping-basket").with_test_name("test_translation"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(
                95,
                5,
                vec![("parse".to_string(), 3), ("content".to_string(), 2)],
            ),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .warning(Warning::new("BASELINE_EXPIRED", "Baseline is 45 days old"))
        .build();

        assert!(record.statistical_analysis().is_some());
        let stats = record.statistical_analysis().unwrap();
        assert!((stats.threshold() - 0.90).abs() < 1e-10);
        assert_eq!(stats.test_statistic(), Some(2.29));

        assert!(record.spec_provenance().is_some());
        let prov = record.spec_provenance().unwrap();
        assert_eq!(prov.spec_filename(), Some("shopping-basket.yaml"));

        assert_eq!(record.warnings().len(), 1);
    }

    #[test]
    fn functional_dimension_computes_pass_rate() {
        let dim = FunctionalDimension::new(80, 20, vec![]);
        assert!((dim.pass_rate() - 0.80).abs() < 1e-10);
    }

    #[test]
    fn functional_dimension_zero_samples() {
        let dim = FunctionalDimension::new(0, 0, vec![]);
        assert!((dim.pass_rate()).abs() < 1e-10);
    }
}
