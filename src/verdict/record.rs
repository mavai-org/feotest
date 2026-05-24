//! The verdict record: the single source of truth for all verdict rendering.

use serde::Serialize;

use crate::latency::LatencyDimension;
use crate::model::{
    ExecutionSummary, ExpirationInfo, PacingSummary, TestIdentity, TestIntent, ThresholdOrigin,
    Warning,
};
use crate::verdict::{FunctionalAssessment, Verdict};

/// The complete record of a probabilistic test verdict.
///
/// This is consumed by all rendering paths: `JUnit` XML, HTML reports,
/// console output, and sentinel verdict sinks. Serialises as a `camelCase`
/// JSON object — the wire shape consumed by file and webhook sinks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerdictRecord {
    identity: TestIdentity,
    verdict: Verdict,
    verdict_reason: String,
    intent: TestIntent,
    execution: ExecutionSummary,
    functional: FunctionalDimension,
    #[serde(skip_serializing_if = "Option::is_none")]
    functional_assessment: Option<FunctionalAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    statistical_analysis: Option<StatisticalAnalysis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spec_provenance: Option<SpecProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_provenance: Option<BaselineProvenance>,
    covariate_status: CovariateStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<Warning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency: Option<LatencyDimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pacing: Option<PacingSummary>,
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_environment"
    )]
    environment: Vec<(String, String)>,
}

/// Serialises the environment metadata as a JSON object (key/value map)
/// rather than an array of tuples. Callers reading the wire shape expect
/// an object they can index by key.
fn serialize_environment<S: serde::Serializer>(
    entries: &[(String, String)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(entries.len()))?;
    for (k, v) in entries {
        map.serialize_entry(k, v)?;
    }
    map.end()
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
            functional_assessment: None,
            statistical_analysis: None,
            spec_provenance: None,
            baseline_provenance: None,
            covariate_status: CovariateStatus::all_aligned(),
            warnings: Vec::new(),
            latency: None,
            correlation_id: None,
            pacing: None,
            environment: Vec::new(),
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

    /// The composite, per-criterion functional assessment, if one was built.
    ///
    /// Carried alongside [`functional`](Self::functional) while the spine is
    /// re-modelled; the single-criterion path populates one row.
    #[must_use]
    pub const fn functional_assessment(&self) -> Option<&FunctionalAssessment> {
        self.functional_assessment.as_ref()
    }

    /// Statistical analysis, if performed.
    #[must_use]
    pub const fn statistical_analysis(&self) -> Option<&StatisticalAnalysis> {
        self.statistical_analysis.as_ref()
    }

    /// The human-readable reason for the verdict.
    #[must_use]
    pub fn verdict_reason(&self) -> &str {
        &self.verdict_reason
    }

    /// Baseline provenance, if a spec was used.
    #[must_use]
    pub const fn spec_provenance(&self) -> Option<&SpecProvenance> {
        self.spec_provenance.as_ref()
    }

    /// Baseline measurement provenance, if a baseline was used.
    #[must_use]
    pub const fn baseline_provenance(&self) -> Option<&BaselineProvenance> {
        self.baseline_provenance.as_ref()
    }

    /// Covariate alignment status.
    #[must_use]
    pub const fn covariate_status(&self) -> &CovariateStatus {
        &self.covariate_status
    }

    /// Warnings attached to this verdict.
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// The latency dimension, if any thresholds were declared or a baseline
    /// latency block was present.
    #[must_use]
    pub const fn latency(&self) -> Option<&LatencyDimension> {
        self.latency.as_ref()
    }

    /// Correlation ID for tracing, if set.
    #[must_use]
    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    /// Pacing summary, if pacing was configured.
    #[must_use]
    pub const fn pacing(&self) -> Option<&PacingSummary> {
        self.pacing.as_ref()
    }

    /// Environment metadata entries.
    #[must_use]
    pub fn environment(&self) -> &[(String, String)] {
        &self.environment
    }

    /// Whether the overall verdict passed.
    ///
    /// Combines the functional verdict with the latency dimension when
    /// present. Advisory latency violations never affect this result.
    #[must_use]
    pub fn passed(&self) -> bool {
        let functional_ok = self.verdict == Verdict::Pass;
        let latency_ok = self.latency.as_ref().is_none_or(LatencyDimension::passed);
        functional_ok && latency_ok
    }

    /// Panics if the functional dimension did not pass.
    ///
    /// # Panics
    ///
    /// Panics with a diagnostic message when the functional verdict is not
    /// `Verdict::Pass`.
    pub fn assert_contract(&self) {
        assert!(
            self.verdict == Verdict::Pass,
            "functional contract failed: verdict = {}",
            self.verdict
        );
    }

    /// Panics if the latency dimension recorded any strict violation.
    ///
    /// No-op when no latency dimension is attached or when the dimension
    /// passed (including advisory-only violations).
    ///
    /// # Panics
    ///
    /// Panics with a diagnostic message listing strict violations when the
    /// latency dimension has any.
    pub fn assert_latency(&self) {
        if let Some(dim) = self.latency.as_ref() {
            assert!(
                dim.passed(),
                "latency contract failed ({} strict violation(s)):\n{}",
                dim.strict_violations(),
                dim
            );
        }
    }

    /// Panics if either dimension failed.
    pub fn assert_all(&self) {
        self.assert_contract();
        self.assert_latency();
    }
}

/// Builder for [`VerdictRecord`].
pub struct VerdictRecordBuilder {
    identity: TestIdentity,
    verdict: Verdict,
    intent: TestIntent,
    execution: ExecutionSummary,
    functional: FunctionalDimension,
    functional_assessment: Option<FunctionalAssessment>,
    statistical_analysis: Option<StatisticalAnalysis>,
    spec_provenance: Option<SpecProvenance>,
    baseline_provenance: Option<BaselineProvenance>,
    covariate_status: CovariateStatus,
    warnings: Vec<Warning>,
    latency: Option<LatencyDimension>,
    correlation_id: Option<String>,
    pacing: Option<PacingSummary>,
    environment: Vec<(String, String)>,
}

impl VerdictRecordBuilder {
    /// Attaches the composite, per-criterion functional assessment.
    #[must_use]
    pub fn functional_assessment(mut self, assessment: FunctionalAssessment) -> Self {
        self.functional_assessment = Some(assessment);
        self
    }

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

    /// Attaches baseline provenance.
    #[must_use]
    pub fn baseline_provenance(mut self, provenance: BaselineProvenance) -> Self {
        self.baseline_provenance = Some(provenance);
        self
    }

    /// Sets the covariate alignment status.
    #[must_use]
    pub fn covariate_status(mut self, status: CovariateStatus) -> Self {
        self.covariate_status = status;
        self
    }

    /// Attaches a latency dimension.
    #[must_use]
    pub fn latency(mut self, dimension: LatencyDimension) -> Self {
        self.latency = Some(dimension);
        self
    }

    /// Sets a correlation ID for tracing.
    #[must_use]
    pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Attaches a pacing summary.
    #[must_use]
    pub const fn pacing(mut self, summary: PacingSummary) -> Self {
        self.pacing = Some(summary);
        self
    }

    /// Sets environment metadata entries.
    #[must_use]
    pub fn environment(mut self, entries: Vec<(String, String)>) -> Self {
        self.environment = entries;
        self
    }

    /// Builds the verdict record.
    ///
    /// The `verdict_reason` field is derived automatically from the verdict,
    /// execution, covariate status, and statistical analysis.
    #[must_use]
    pub fn build(self) -> VerdictRecord {
        let verdict_reason = derive_verdict_reason(
            self.verdict,
            &self.execution,
            &self.covariate_status,
            &self.functional,
            self.statistical_analysis.as_ref(),
        );
        VerdictRecord {
            identity: self.identity,
            verdict: self.verdict,
            verdict_reason,
            intent: self.intent,
            execution: self.execution,
            functional: self.functional,
            functional_assessment: self.functional_assessment,
            statistical_analysis: self.statistical_analysis,
            spec_provenance: self.spec_provenance,
            baseline_provenance: self.baseline_provenance,
            covariate_status: self.covariate_status,
            warnings: self.warnings,
            latency: self.latency,
            correlation_id: self.correlation_id,
            pacing: self.pacing,
            environment: self.environment,
        }
    }
}

/// Functional dimension of a verdict: success/failure counts and pass rate.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionalDimension {
    successes: u32,
    failures: u32,
    pass_rate: f64,
    #[serde(serialize_with = "serialize_string_u32_pairs")]
    failure_distribution: Vec<(String, u32)>,
    conformance_mismatches: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    example_mismatches: Vec<String>,
}

fn serialize_string_u32_pairs<S: serde::Serializer>(
    pairs: &[(String, u32)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(pairs.len()))?;
    for (k, v) in pairs {
        map.serialize_entry(k, v)?;
    }
    map.end()
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatisticalAnalysis {
    confidence_level: f64,
    standard_error: f64,
    wilson_lower: f64,
    threshold: f64,
    threshold_origin: ThresholdOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_statistic: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    p_value: Option<f64>,
}

impl StatisticalAnalysis {
    /// Creates a new statistical analysis.
    #[must_use]
    pub const fn new(
        confidence_level: f64,
        standard_error: f64,
        wilson_lower: f64,
        threshold: f64,
        threshold_origin: ThresholdOrigin,
    ) -> Self {
        Self {
            confidence_level,
            standard_error,
            wilson_lower,
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

    /// Wilson one-sided lower bound at the verdict's confidence level.
    ///
    /// The verdict path is left-tailed (degradation only); the upper
    /// bound carries no operational meaning here and is therefore not
    /// retained.
    #[must_use]
    pub const fn wilson_lower(&self) -> f64 {
        self.wilson_lower
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

/// Covariate alignment status between the baseline and the observed run.
///
/// When no covariates are declared, both profiles are empty and `aligned`
/// is `true`. When covariates are declared but all values match, `aligned`
/// is `true` and `misalignments` is empty.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CovariateStatus {
    aligned: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    misalignments: Vec<Misalignment>,
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_string_string_pairs"
    )]
    baseline_profile: Vec<(String, String)>,
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_string_string_pairs"
    )]
    observed_profile: Vec<(String, String)>,
}

fn serialize_string_string_pairs<S: serde::Serializer>(
    pairs: &[(String, String)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(pairs.len()))?;
    for (k, v) in pairs {
        map.serialize_entry(k, v)?;
    }
    map.end()
}

impl CovariateStatus {
    /// Creates a covariate status from profiles and computed misalignments.
    #[must_use]
    pub const fn new(
        aligned: bool,
        misalignments: Vec<Misalignment>,
        baseline_profile: Vec<(String, String)>,
        observed_profile: Vec<(String, String)>,
    ) -> Self {
        Self {
            aligned,
            misalignments,
            baseline_profile,
            observed_profile,
        }
    }

    /// Creates a status indicating all covariates are aligned (or none declared).
    #[must_use]
    pub const fn all_aligned() -> Self {
        Self {
            aligned: true,
            misalignments: Vec::new(),
            baseline_profile: Vec::new(),
            observed_profile: Vec::new(),
        }
    }

    /// Whether all covariates are aligned.
    #[must_use]
    pub const fn aligned(&self) -> bool {
        self.aligned
    }

    /// Individual misalignments, if any.
    #[must_use]
    pub fn misalignments(&self) -> &[Misalignment] {
        &self.misalignments
    }

    /// Covariate key-value pairs from the baseline.
    #[must_use]
    pub fn baseline_profile(&self) -> &[(String, String)] {
        &self.baseline_profile
    }

    /// Covariate key-value pairs observed at test time.
    #[must_use]
    pub fn observed_profile(&self) -> &[(String, String)] {
        &self.observed_profile
    }
}

/// A single covariate key whose baseline and observed values differ.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Misalignment {
    key: String,
    baseline_value: String,
    observed_value: String,
}

impl Misalignment {
    /// Creates a new misalignment record.
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        baseline_value: impl Into<String>,
        observed_value: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            baseline_value: baseline_value.into(),
            observed_value: observed_value.into(),
        }
    }

    /// The covariate key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The value recorded in the baseline.
    #[must_use]
    pub fn baseline_value(&self) -> &str {
        &self.baseline_value
    }

    /// The value observed at test time.
    #[must_use]
    pub fn observed_value(&self) -> &str {
        &self.observed_value
    }
}

/// Provenance of the baseline measurement used for threshold derivation.
///
/// Carries enough data to render the baseline provenance block in the
/// console output: which file, when it was generated, and the key
/// statistical parameters.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineProvenance {
    source_file: String,
    generated_at: String,
    baseline_samples: u32,
    baseline_rate: f64,
    derived_threshold: f64,
}

impl BaselineProvenance {
    /// Creates a new baseline provenance record.
    #[must_use]
    pub fn new(
        source_file: impl Into<String>,
        generated_at: impl Into<String>,
        baseline_samples: u32,
        baseline_rate: f64,
        derived_threshold: f64,
    ) -> Self {
        Self {
            source_file: source_file.into(),
            generated_at: generated_at.into(),
            baseline_samples,
            baseline_rate,
            derived_threshold,
        }
    }

    /// The baseline spec filename.
    #[must_use]
    pub fn source_file(&self) -> &str {
        &self.source_file
    }

    /// ISO 8601 timestamp of when the baseline was generated.
    #[must_use]
    pub fn generated_at(&self) -> &str {
        &self.generated_at
    }

    /// Number of samples in the baseline measurement.
    #[must_use]
    pub const fn baseline_samples(&self) -> u32 {
        self.baseline_samples
    }

    /// Observed success rate in the baseline measurement.
    #[must_use]
    pub const fn baseline_rate(&self) -> f64 {
        self.baseline_rate
    }

    /// The threshold derived from the baseline.
    #[must_use]
    pub const fn derived_threshold(&self) -> f64 {
        self.derived_threshold
    }
}

/// Provenance of the baseline spec used for threshold derivation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecProvenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    spec_filename: Option<String>,
    threshold_origin: ThresholdOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    contract_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expiration: Option<ExpirationInfo>,
}

impl SpecProvenance {
    /// Creates spec provenance.
    #[must_use]
    pub const fn new(threshold_origin: ThresholdOrigin) -> Self {
        Self {
            spec_filename: None,
            threshold_origin,
            contract_ref: None,
            expiration: None,
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

    /// Sets expiration info on this provenance.
    #[must_use]
    pub fn with_expiration(mut self, info: ExpirationInfo) -> Self {
        self.expiration = Some(info);
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

    /// Expiration info for the baseline spec, if any.
    #[must_use]
    pub const fn expiration(&self) -> Option<&ExpirationInfo> {
        self.expiration.as_ref()
    }
}

/// Derives the verdict reason from the verdict, execution, and analysis context.
fn derive_verdict_reason(
    verdict: Verdict,
    execution: &ExecutionSummary,
    covariate_status: &CovariateStatus,
    functional: &FunctionalDimension,
    analysis: Option<&StatisticalAnalysis>,
) -> String {
    let is_budget_exhausted = execution.termination().reason().is_budget_exhausted();

    match verdict {
        Verdict::Pass => {
            let observed = functional.pass_rate();
            let threshold = analysis.map_or(0.0, StatisticalAnalysis::threshold);
            format!("{observed:.4} >= {threshold:.4}")
        }
        Verdict::Fail => {
            if is_budget_exhausted {
                "budget exhausted".to_string()
            } else {
                let observed = functional.pass_rate();
                let threshold = analysis.map_or(0.0, StatisticalAnalysis::threshold);
                format!("{observed:.4} < {threshold:.4}")
            }
        }
        Verdict::Inconclusive => {
            if !covariate_status.aligned() {
                "covariate misalignment".to_string()
            } else if is_budget_exhausted {
                "budget exhausted".to_string()
            } else {
                "insufficient evidence".to_string()
            }
        }
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
        assert_eq!(record.identity().service_contract_id(), "shopping-basket");
        assert!(record.statistical_analysis().is_none());
        assert!(record.spec_provenance().is_none());
        assert!(record.warnings().is_empty());
    }

    #[test]
    fn builds_full_verdict_record() {
        let analysis =
            StatisticalAnalysis::new(0.95, 0.0218, 0.9073, 0.90, ThresholdOrigin::Empirical)
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

    // --- Verdict reason derivation ---

    #[test]
    fn verdict_reason_pass() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .statistical_analysis(StatisticalAnalysis::new(
            0.95,
            0.022,
            0.907,
            0.900,
            ThresholdOrigin::Empirical,
        ))
        .build();

        assert_eq!(record.verdict_reason(), "0.9500 >= 0.9000");
    }

    #[test]
    fn verdict_reason_fail_completed() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(80, 20, vec![]),
        )
        .statistical_analysis(StatisticalAnalysis::new(
            0.95,
            0.040,
            0.722,
            0.900,
            ThresholdOrigin::Empirical,
        ))
        .build();

        assert_eq!(record.verdict_reason(), "0.8000 < 0.9000");
    }

    #[test]
    fn verdict_reason_fail_budget_exhausted() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Fail,
            TestIntent::Verification,
            ExecutionSummary::new(
                100,
                50,
                30,
                20,
                TerminationInfo::new(TerminationReason::TimeBudgetExhausted),
                CostSummary::new(Duration::from_secs(60), 0, 50),
            ),
            FunctionalDimension::new(30, 20, vec![]),
        )
        .build();

        assert_eq!(record.verdict_reason(), "budget exhausted");
    }

    #[test]
    fn verdict_reason_inconclusive_covariate_misalignment() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Inconclusive,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(85, 15, vec![]),
        )
        .covariate_status(CovariateStatus::new(
            false,
            vec![Misalignment::new("model", "gpt-4o", "gpt-3.5")],
            vec![("model".to_string(), "gpt-4o".to_string())],
            vec![("model".to_string(), "gpt-3.5".to_string())],
        ))
        .build();

        assert_eq!(record.verdict_reason(), "covariate misalignment");
    }

    #[test]
    fn verdict_reason_inconclusive_budget_exhausted() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Inconclusive,
            TestIntent::Verification,
            ExecutionSummary::new(
                100,
                20,
                15,
                5,
                TerminationInfo::new(TerminationReason::TokenBudgetExhausted),
                CostSummary::new(Duration::from_secs(10), 10_000, 20),
            ),
            FunctionalDimension::new(15, 5, vec![]),
        )
        .build();

        assert_eq!(record.verdict_reason(), "budget exhausted");
    }

    #[test]
    fn verdict_reason_inconclusive_insufficient_evidence() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Inconclusive,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(7, 3, vec![]),
        )
        .build();

        assert_eq!(record.verdict_reason(), "insufficient evidence");
    }

    // --- New field accessors ---

    #[test]
    fn covariate_status_defaults_to_aligned() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .build();

        assert!(record.covariate_status().aligned());
        assert!(record.covariate_status().misalignments().is_empty());
    }

    #[test]
    fn baseline_provenance_defaults_to_none() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .build();

        assert!(record.baseline_provenance().is_none());
    }

    #[test]
    fn baseline_provenance_set_and_readable() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .baseline_provenance(BaselineProvenance::new(
            "test.yaml",
            "2026-04-01T12:00:00Z",
            200,
            0.95,
            0.90,
        ))
        .build();

        let bp = record.baseline_provenance().unwrap();
        assert_eq!(bp.source_file(), "test.yaml");
        assert_eq!(bp.generated_at(), "2026-04-01T12:00:00Z");
        assert_eq!(bp.baseline_samples(), 200);
        assert!((bp.baseline_rate() - 0.95).abs() < 1e-10);
        assert!((bp.derived_threshold() - 0.90).abs() < 1e-10);
    }

    #[test]
    fn new_fields_default_to_none_or_empty() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .build();

        assert!(record.correlation_id().is_none());
        assert!(record.pacing().is_none());
        assert!(record.environment().is_empty());
    }

    #[test]
    fn new_fields_set_and_readable() {
        use crate::controls::PacingConfig;
        use crate::model::{ExpirationInfo, ExpirationStatus, PacingSummary};

        let pacing = PacingSummary::from_config(&PacingConfig::new().max_requests_per_second(10.0));

        let provenance =
            SpecProvenance::new(ThresholdOrigin::Empirical).with_expiration(ExpirationInfo::new(
                ExpirationStatus::ExpiringSoon,
                Some("2026-06-01T00:00:00Z".into()),
            ));

        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalDimension::new(95, 5, vec![]),
        )
        .correlation_id("run-123")
        .pacing(pacing)
        .environment(vec![("region".to_string(), "eu-west-1".to_string())])
        .spec_provenance(provenance)
        .build();

        assert_eq!(record.correlation_id(), Some("run-123"));
        assert!(record.pacing().is_some());
        assert_eq!(record.pacing().unwrap().effective_min_delay_ms(), 100);
        assert_eq!(record.environment().len(), 1);
        assert_eq!(record.environment()[0].0, "region");

        let exp = record.spec_provenance().unwrap().expiration().unwrap();
        assert_eq!(exp.status(), &ExpirationStatus::ExpiringSoon);
        assert_eq!(exp.expires_at(), Some("2026-06-01T00:00:00Z"));
    }

    #[test]
    fn conformance_dimension_accessors() {
        let dim = FunctionalDimension::new(80, 20, vec![("parse".to_string(), 12)])
            .conformance(3, vec!["diff1".to_string(), "diff2".to_string()]);
        assert_eq!(dim.conformance_mismatches(), 3);
        assert_eq!(dim.example_mismatches().len(), 2);
        assert_eq!(dim.failure_distribution().len(), 1);
    }
}
