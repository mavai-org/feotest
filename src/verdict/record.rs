//! The verdict record: the single source of truth for all verdict rendering.

use serde::Serialize;

use crate::latency::{LatencyDimension, LatencyEvaluation};
use crate::model::{
    ExecutionSummary, ExpirationInfo, PacingSummary, TestIdentity, TestIntent, ThresholdOrigin,
    Warning,
};
use crate::statistics::decision::{
    ComplianceDecision, Envelopes, RegressionDecision, Trigger, TriggerKind, type_one_envelopes,
};
use crate::statistics::proportion;
use crate::statistics::rules::{
    ConfigurationError, DecisionRule, EnforcementMode, METHODOLOGY_VERSION,
};
use crate::statistics::types::ConfidenceLevel;
use crate::verdict::{CriterionRow, FunctionalAssessment, Verdict};

/// The complete record of a probabilistic test verdict.
///
/// This is consumed by all rendering paths: `JUnit` XML, HTML reports,
/// console output, and sentinel verdict sinks. Serialises as a `camelCase`
/// JSON object — the wire shape consumed by file and webhook sinks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
// mavai-ref: JVI-NSB1JPC — do not remove (resolves in mavai-orchestrator)
pub struct VerdictRecord {
    identity: TestIdentity,
    methodology_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<Verdict>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    configuration_errors: Vec<ConfigurationError>,
    verdict_reason: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    triggering: Vec<Trigger>,
    intent: TestIntent,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence_level: Option<f64>,
    execution: ExecutionSummary,
    functional_assessment: FunctionalAssessment,
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
    /// Starts building a verdict record for a test that ran and was decided.
    ///
    /// `verdict` is the overall test verdict `V_test`: the structural
    /// composite of the enforced dimensions among the functional criteria
    /// and the latency constraints (PASS when none is enforced).
    #[must_use]
    pub const fn builder(
        identity: TestIdentity,
        verdict: Verdict,
        intent: TestIntent,
        execution: ExecutionSummary,
        functional_assessment: FunctionalAssessment,
    ) -> VerdictRecordBuilder {
        VerdictRecordBuilder {
            identity,
            verdict: Some(verdict),
            configuration_errors: Vec::new(),
            triggering: Vec::new(),
            intent,
            confidence_level: None,
            execution,
            functional_assessment,
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

    /// Starts building the record of a configuration refused before any
    /// sample ran: it carries every applicable configuration error, in the
    /// fixed order, and no verdict — a refusal is not INCONCLUSIVE, which
    /// is reserved for outcomes that depend on the data.
    ///
    /// # Panics
    ///
    /// Panics if `errors` is empty — a refusal names at least one error.
    #[must_use]
    pub fn refused(
        identity: TestIdentity,
        intent: TestIntent,
        execution: ExecutionSummary,
        errors: Vec<ConfigurationError>,
    ) -> VerdictRecordBuilder {
        assert!(
            !errors.is_empty(),
            "a refused configuration names at least one configuration error"
        );
        VerdictRecordBuilder {
            identity,
            verdict: None,
            configuration_errors: crate::statistics::rules::ordered_configuration_errors(errors),
            triggering: Vec::new(),
            intent,
            confidence_level: None,
            execution,
            functional_assessment: FunctionalAssessment::refused(),
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

    /// The methodology whose decision rules produced the verdict.
    #[must_use]
    pub const fn methodology_version(&self) -> &'static str {
        self.methodology_version
    }

    /// The overall test verdict `V_test`; `None` when the configuration was
    /// refused before any sample ran.
    #[must_use]
    pub const fn verdict(&self) -> Option<Verdict> {
        self.verdict
    }

    /// Every configuration error that refused the run, in the fixed order;
    /// empty for a run that was decided.
    #[must_use]
    pub fn configuration_errors(&self) -> &[ConfigurationError] {
        &self.configuration_errors
    }

    /// Whether the configuration was refused before any sample ran.
    #[must_use]
    pub const fn is_refused(&self) -> bool {
        self.verdict.is_none()
    }

    /// For a FAIL or an INCONCLUSIVE, the functional criteria and latency
    /// constraints of the enforced dimensions whose verdict is the test's,
    /// criteria first.
    #[must_use]
    pub fn triggering(&self) -> &[Trigger] {
        &self.triggering
    }

    /// The rule that decided the whole test, when exactly one did: every
    /// judged criterion and latency evaluation of the enforced dimensions was
    /// decided by it. `None` when several rules decided (each is stated where
    /// it decided) or none did; an advisory dimension's rules do not count.
    #[must_use]
    pub fn single_decision_rule(&self) -> Option<DecisionRule> {
        let mut rules = self.decisions().map(|(rule, _)| rule);
        let first = rules.next()?;
        rules.all(|rule| rule == first).then_some(first)
    }

    /// The union-bound Type-I envelopes over the binding decisions the test
    /// made, split by direction (false compliance, false degradation
    /// signal); an advisory dimension's decisions add nothing.
    #[must_use]
    pub fn envelopes(&self) -> Envelopes {
        type_one_envelopes(self.decisions())
    }

    /// Every binding `(rule, alpha)` decision: the judged criteria, then
    /// the latency evaluations, of the enforced dimensions only.
    fn decisions(&self) -> impl Iterator<Item = (DecisionRule, f64)> + '_ {
        let criteria = self
            .functional_assessment
            .criteria()
            .iter()
            .filter(|_| self.functional_assessment.mode().is_enforced())
            .filter_map(CriterionRow::statistical_analysis)
            .map(|analysis| (analysis.decision_rule(), analysis.alpha()));
        let latency = self
            .latency
            .iter()
            .filter(|dimension| dimension.mode().is_enforced())
            .flat_map(LatencyDimension::decisions);
        criteria.chain(latency)
    }

    /// The declared test intent.
    #[must_use]
    pub const fn intent(&self) -> TestIntent {
        self.intent
    }

    /// The test's confidence level (`1 − alpha`), when the record states
    /// one; a criterion may be decided at its own.
    #[must_use]
    pub const fn confidence_level(&self) -> Option<f64> {
        self.confidence_level
    }

    /// Execution summary (samples, timing, termination).
    #[must_use]
    pub const fn execution(&self) -> &ExecutionSummary {
        &self.execution
    }

    /// The composite, per-criterion functional assessment: the per-criterion
    /// rows and the composite verdict over them. The single-criterion path
    /// populates exactly one row.
    #[must_use]
    pub const fn functional_assessment(&self) -> &FunctionalAssessment {
        &self.functional_assessment
    }

    /// The first criterion row, for renderers that show one functional
    /// figure; `None` for a refused configuration, which judged nothing.
    #[must_use]
    pub(crate) fn functional_summary(&self) -> Option<&CriterionRow> {
        self.functional_assessment.criteria().first()
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

    /// Whether the overall test verdict `V_test` is PASS.
    ///
    /// `V_test` composes the enforced dimensions only; an advisory
    /// dimension's verdict never affects it. A refused configuration has not
    /// passed.
    #[must_use]
    // mavai-ref: JVI-ZCSHQ5K — do not remove (resolves in mavai-orchestrator)
    pub fn passed(&self) -> bool {
        self.verdict == Some(Verdict::Pass)
    }

    /// Panics if the functional dimension `V_rate` binds the test and did
    /// not pass.
    ///
    /// When the run makes the functional dimension advisory, `V_rate` is
    /// decided and recorded but never panics this method.
    ///
    /// # Panics
    ///
    /// Panics with a diagnostic message when the configuration was refused,
    /// whatever the run's modes, or when the enforced functional composite
    /// is not PASS.
    // mavai-ref: JVI-Y3710A7 — do not remove (resolves in mavai-orchestrator)
    pub fn assert_contract(&self) {
        self.assert_not_refused();
        if let Some(failure) = self.functional_failure() {
            panic!("{failure}{}", self.advisory_context());
        }
    }

    /// Panics if the latency dimension `V_latency` binds the test and is
    /// FAIL or INCONCLUSIVE.
    ///
    /// A no-op when the test carries no latency constraint; when the run
    /// makes the latency dimension advisory, `V_latency` is decided and
    /// recorded but never panics this method.
    ///
    /// # Panics
    ///
    /// Panics with a diagnostic message when the configuration was refused,
    /// whatever the run's modes, or when the enforced latency constraints
    /// did not pass.
    // mavai-ref: JVI-Y3710A7 — do not remove (resolves in mavai-orchestrator)
    pub fn assert_latency(&self) {
        self.assert_not_refused();
        if let Some(failure) = self.latency_failure() {
            panic!("{failure}{}", self.advisory_context());
        }
    }

    /// Panics unless every enforced dimension passed: `assert_contract` and
    /// `assert_latency` together, with one message.
    ///
    /// # Panics
    ///
    /// Panics when the configuration was refused, whatever the run's modes,
    /// or when an enforced dimension did not pass.
    // mavai-ref: JVI-Y3710A7 — do not remove (resolves in mavai-orchestrator)
    pub fn assert_all(&self) {
        self.assert_not_refused();
        let failures: Vec<String> = self
            .functional_failure()
            .into_iter()
            .chain(self.latency_failure())
            .collect();
        assert!(
            failures.is_empty(),
            "{}{}",
            failures.join("\n"),
            self.advisory_context()
        );
    }

    /// Panics with the configuration errors when the run was refused.
    fn assert_not_refused(&self) {
        assert!(
            !self.is_refused(),
            "configuration refused before any sample ran: {}",
            self.verdict_reason
        );
    }

    /// The failure message of an enforced functional dimension that did not
    /// pass; `None` when it passed or is advisory.
    fn functional_failure(&self) -> Option<String> {
        let assessment = &self.functional_assessment;
        let composite = assessment.composite();
        (assessment.mode().is_enforced() && composite != Verdict::Pass)
            .then(|| format!("functional contract failed: verdict = {composite}"))
    }

    /// The failure message of an enforced latency dimension that did not
    /// pass; `None` when it passed, is advisory or is absent.
    fn latency_failure(&self) -> Option<String> {
        let dimension = self.latency.as_ref()?;
        (dimension.mode().is_enforced() && !dimension.passed()).then(|| {
            let decided: Vec<&str> = dimension
                .evaluations()
                .iter()
                .filter(|e| e.verdict() != Verdict::Pass)
                .map(LatencyEvaluation::constraint_id)
                .collect();
            format!(
                "latency contract failed (not passed: {}):\n{dimension}",
                decided.join(", ")
            )
        })
    }

    /// The verdicts of the advisory dimensions, listed for context beside a
    /// failure and never as its cause; empty when no dimension is advisory.
    fn advisory_context(&self) -> String {
        let mut lines = Vec::new();
        if self.functional_assessment.mode() == EnforcementMode::Advisory {
            lines.push(format!(
                "\nadvisory (does not fail the test): functional verdict = {}",
                self.functional_assessment.composite()
            ));
        }
        if let Some(dimension) = self
            .latency
            .as_ref()
            .filter(|dimension| dimension.mode() == EnforcementMode::Advisory)
            && let Some(verdict) = dimension.verdict()
        {
            lines.push(format!(
                "\nadvisory (does not fail the test): latency verdict = {verdict}"
            ));
        }
        lines.concat()
    }
}

/// Builder for [`VerdictRecord`].
pub struct VerdictRecordBuilder {
    identity: TestIdentity,
    verdict: Option<Verdict>,
    configuration_errors: Vec<ConfigurationError>,
    triggering: Vec<Trigger>,
    intent: TestIntent,
    confidence_level: Option<f64>,
    execution: ExecutionSummary,
    functional_assessment: FunctionalAssessment,
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
    /// Names the criteria and latency constraints of the enforced dimensions
    /// that decided a FAIL or an INCONCLUSIVE.
    #[must_use]
    pub fn triggering(mut self, triggering: Vec<Trigger>) -> Self {
        self.triggering = triggering;
        self
    }

    /// States the test's confidence level (`1 − alpha`).
    #[must_use]
    pub const fn confidence_level(mut self, confidence_level: f64) -> Self {
        self.confidence_level = Some(confidence_level);
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
        let verdict_reason = self.verdict.map_or_else(
            || refusal_reason(&self.configuration_errors),
            |verdict| {
                derive_verdict_reason(
                    verdict,
                    &self.execution,
                    &self.covariate_status,
                    &self.functional_assessment,
                    self.latency.as_ref(),
                    &self.triggering,
                )
            },
        );
        VerdictRecord {
            identity: self.identity,
            methodology_version: METHODOLOGY_VERSION,
            verdict: self.verdict,
            configuration_errors: self.configuration_errors,
            verdict_reason,
            triggering: self.triggering,
            intent: self.intent,
            confidence_level: self.confidence_level,
            execution: self.execution,
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

/// The statistics behind one criterion's verdict: the rule that decided it,
/// the evidence the rule computed, and the descriptive context reported
/// beside it.
///
/// The Wilson lower bound and the standard error are descriptive only — no
/// rule decides with them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatisticalAnalysis {
    confidence_level: f64,
    alpha: f64,
    standard_error: f64,
    wilson_lower: f64,
    threshold: f64,
    threshold_origin: ThresholdOrigin,
    decision_rule: DecisionRule,
    decision_rule_version: u32,
    evidence: RuleEvidence,
}

impl StatisticalAnalysis {
    /// Creates the statistical analysis of one decided criterion.
    ///
    /// `threshold` is the bar as a rate: the requirement for a compliance
    /// decision, the cutoff over the test size for a regression decision.
    #[must_use]
    pub fn new(
        confidence_level: f64,
        standard_error: f64,
        wilson_lower: f64,
        threshold: f64,
        threshold_origin: ThresholdOrigin,
        evidence: RuleEvidence,
    ) -> Self {
        let decision_rule = evidence.rule();
        Self {
            confidence_level,
            alpha: crate::statistics::rules::alpha_from_confidence(confidence_level),
            standard_error,
            wilson_lower,
            threshold,
            threshold_origin,
            decision_rule,
            decision_rule_version: decision_rule.version(),
            evidence,
        }
    }

    /// The analysis of a criterion decided by `compliance/exact-binomial`,
    /// with its descriptive Wilson context.
    #[must_use]
    pub fn compliance(
        decision: &ComplianceDecision,
        confidence_level: f64,
        threshold_origin: ThresholdOrigin,
    ) -> Self {
        Self::new(
            confidence_level,
            proportion::standard_error(decision.successes(), decision.trials()),
            proportion::lower_bound(
                decision.successes(),
                decision.trials(),
                ConfidenceLevel::new(confidence_level),
            ),
            decision.requirement(),
            threshold_origin,
            RuleEvidence::Compliance(ComplianceEvidence {
                requirement: decision.requirement(),
                minimum_passing_count: decision.minimum_passing(),
                false_compliance: decision.false_compliance(),
                clopper_pearson_lower: decision.clopper_pearson_lower(),
            }),
        )
    }

    /// The analysis of a criterion decided by `regression/fisher`, with its
    /// descriptive Wilson context and what the design can detect.
    #[must_use]
    pub fn regression(
        decision: &RegressionDecision,
        confidence_level: f64,
        disclosure: DesignDisclosure,
    ) -> Self {
        let derivation = decision.derivation();
        Self::new(
            confidence_level,
            proportion::standard_error(decision.successes(), decision.trials()),
            proportion::lower_bound(
                decision.successes(),
                decision.trials(),
                ConfidenceLevel::new(confidence_level),
            ),
            derivation.threshold_real(),
            ThresholdOrigin::Empirical,
            RuleEvidence::Regression(RegressionEvidence {
                baseline_successes: decision.baseline_successes(),
                baseline_trials: decision.baseline_trials(),
                cutoff: derivation.cutoff(),
                size_at_assumed_common_rate: derivation.size_at_assumed_common_rate(),
                minimum_detectable_degradation: disclosure.minimum_detectable_degradation,
                design_alternative_rate: disclosure.design_alternative_rate,
                design_power: disclosure.design_power,
                resolved_test_power: disclosure.resolved_test_power,
            }),
        )
    }

    /// The confidence level used.
    #[must_use]
    pub const fn confidence_level(&self) -> f64 {
        self.confidence_level
    }

    /// The one-sided level of the decision, `1 − confidence`.
    #[must_use]
    pub const fn alpha(&self) -> f64 {
        self.alpha
    }

    /// Standard error of the observed proportion (descriptive).
    #[must_use]
    pub const fn standard_error(&self) -> f64 {
        self.standard_error
    }

    /// Wilson one-sided lower bound at the verdict's confidence level
    /// (descriptive; no rule decides with it).
    #[must_use]
    pub const fn wilson_lower(&self) -> f64 {
        self.wilson_lower
    }

    /// The bar as a rate: the requirement (compliance) or the cutoff over the
    /// test size (regression).
    #[must_use]
    pub const fn threshold(&self) -> f64 {
        self.threshold
    }

    /// Where the threshold came from.
    #[must_use]
    pub const fn threshold_origin(&self) -> ThresholdOrigin {
        self.threshold_origin
    }

    /// The versioned rule that decided the criterion.
    #[must_use]
    pub const fn decision_rule(&self) -> DecisionRule {
        self.decision_rule
    }

    /// What the rule computed.
    #[must_use]
    pub const fn evidence(&self) -> &RuleEvidence {
        &self.evidence
    }

    /// The smallest passing count under the rule that decided the criterion,
    /// as the rule computed it: see [`RuleEvidence::required_pass`].
    #[must_use]
    pub const fn required_pass(&self) -> Option<u32> {
        self.evidence.required_pass()
    }
}

/// What a decision rule computed for one criterion.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(untagged)]
pub enum RuleEvidence {
    /// `compliance/exact-binomial`.
    Compliance(ComplianceEvidence),
    /// `regression/fisher`.
    Regression(RegressionEvidence),
}

impl RuleEvidence {
    /// The rule this evidence belongs to.
    #[must_use]
    pub const fn rule(&self) -> DecisionRule {
        match self {
            Self::Compliance(_) => DecisionRule::ComplianceExactBinomial,
            Self::Regression(_) => DecisionRule::RegressionFisher,
        }
    }

    /// The smallest passing count the rule decided with, so PASS iff the
    /// success count reaches it: the Fisher cutoff for `regression/fisher`,
    /// `k_min` for `compliance/exact-binomial`. `None` when no count can
    /// pass (a compliance design too small). The count is the rule's own,
    /// never recomputed from the threshold rate.
    #[must_use]
    pub const fn required_pass(&self) -> Option<u32> {
        match self {
            Self::Compliance(evidence) => evidence.minimum_passing_count,
            Self::Regression(evidence) => Some(evidence.cutoff),
        }
    }
}

/// The evidence of `compliance/exact-binomial`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplianceEvidence {
    /// The requirement `p_req`.
    pub requirement: f64,
    /// `k_min`, the smallest passing count; `None` when no count can pass.
    pub minimum_passing_count: Option<u32>,
    /// `P_{p_req}(K ≥ k_min)`, 0 when no count can pass.
    pub false_compliance: f64,
    /// The one-sided Clopper–Pearson lower bound (descriptive).
    pub clopper_pearson_lower: f64,
}

/// What a regression design can detect, as a report discloses it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DesignDisclosure {
    /// The smallest degradation the design detects at 80% design power;
    /// `None` when none is detectable.
    pub minimum_detectable_degradation: Option<f64>,
    /// The design alternative rate the run was sized for.
    pub design_alternative_rate: Option<f64>,
    /// The design power at it (baseline and test both yet to be drawn).
    pub design_power: Option<f64>,
    /// The resolved power at it (the observed baseline's cutoff fixed).
    pub resolved_test_power: Option<f64>,
}

/// The evidence of `regression/fisher`, and what the design can detect.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegressionEvidence {
    /// `K_b`, the baseline successes the cutoff was derived from.
    pub baseline_successes: u32,
    /// `n_b`, the baseline trials.
    pub baseline_trials: u32,
    /// The integer cutoff: PASS iff `K_t ≥ cutoff`.
    pub cutoff: u32,
    /// The false-degradation-signal probability were the common rate the
    /// baseline's observed rate — a property of the procedure, not the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_at_assumed_common_rate: Option<f64>,
    /// The smallest degradation the design detects at 80% design power —
    /// the inversion of the design power; `None` when none is detectable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_detectable_degradation: Option<f64>,
    /// The true rate at which the test is to reach its target power, when
    /// the sizing declared one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design_alternative_rate: Option<f64>,
    /// The power at the design alternative rate with the baseline and the
    /// test both yet to be drawn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design_power: Option<f64>,
    /// The power at the design alternative rate of the test resolved against
    /// the observed baseline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_test_power: Option<f64>,
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

/// The reason a refused record states: every configuration error.
fn refusal_reason(errors: &[ConfigurationError]) -> String {
    let codes: Vec<&str> = errors.iter().map(|e| e.code()).collect();
    format!("configuration refused: {}", codes.join(" "))
}

/// Renders the comparison the rule decided on for one criterion row: the
/// success count against the smallest passing count (compliance) or against
/// the cutoff (regression), the relation read from the row's own verdict.
fn judged_comparison(row: Option<&CriterionRow>) -> String {
    let Some(row) = row else {
        return "no criterion judged".to_owned();
    };
    let Some(analysis) = row.statistical_analysis() else {
        return format!("{} of {} observed", row.pass(), row.total());
    };
    let relation = if row.verdict() == Verdict::Pass {
        ">="
    } else {
        "<"
    };
    let bar = match analysis.evidence() {
        RuleEvidence::Compliance(evidence) => match evidence.minimum_passing_count {
            Some(k_min) => format!("k_min {k_min}"),
            None => {
                return format!(
                    "{} of {}: no count of {} can pass ({})",
                    row.pass(),
                    row.total(),
                    row.total(),
                    analysis.decision_rule()
                );
            }
        },
        RuleEvidence::Regression(evidence) => format!("cutoff {}", evidence.cutoff),
    };
    format!(
        "{} of {} {relation} {bar} ({})",
        row.pass(),
        row.total(),
        analysis.decision_rule()
    )
}

/// Names what triggered the verdict: each triggering criterion by the
/// comparison its rule decided on, each latency constraint by its label.
fn triggered_by(rows: &[CriterionRow], triggering: &[Trigger]) -> String {
    let names: Vec<String> = triggering
        .iter()
        .map(|t| match t.kind() {
            TriggerKind::Criterion => rows.iter().find(|row| row.name() == t.id()).map_or_else(
                || format!("criterion {}", t.id()),
                |row| format!("{}: {}", t.id(), judged_comparison(Some(row))),
            ),
            TriggerKind::Latency => format!("latency {}", t.id()),
        })
        .collect();
    names.join("; ")
}

/// Derives the verdict reason from the verdict, execution, and analysis
/// context: what the enforced dimensions decided, followed by the dimensions
/// the run made advisory.
fn derive_verdict_reason(
    verdict: Verdict,
    execution: &ExecutionSummary,
    covariate_status: &CovariateStatus,
    assessment: &FunctionalAssessment,
    latency: Option<&LatencyDimension>,
    triggering: &[Trigger],
) -> String {
    let binding_rows = if assessment.mode().is_enforced() {
        assessment.criteria()
    } else {
        &[]
    };
    let latency_binds = latency.is_some_and(|dimension| dimension.mode().is_enforced());
    let reason = binding_reason(
        verdict,
        execution,
        covariate_status,
        binding_rows,
        latency_binds,
        triggering,
    );
    let advisory: Vec<&str> = [
        (assessment.mode() == EnforcementMode::Advisory).then_some("functional"),
        latency
            .filter(|dimension| dimension.mode() == EnforcementMode::Advisory)
            .map(|_| "latency"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if advisory.is_empty() {
        reason
    } else {
        format!("{reason}; {} advisory", advisory.join(" and "))
    }
}

/// The reason the enforced dimensions give: the comparison the deciding
/// criterion's rule made, or what triggered a FAIL or an INCONCLUSIVE.
/// `binding_rows` is empty when the functional dimension is advisory.
fn binding_reason(
    verdict: Verdict,
    execution: &ExecutionSummary,
    covariate_status: &CovariateStatus,
    binding_rows: &[CriterionRow],
    latency_binds: bool,
    triggering: &[Trigger],
) -> String {
    let is_budget_exhausted = execution.termination().reason().is_budget_exhausted();
    let first_row = binding_rows.first();
    let single_row_trigger = matches!(
        (first_row, triggering),
        (Some(row), [trigger]) if binding_rows.len() == 1
            && trigger.kind() == TriggerKind::Criterion
            && trigger.id() == row.name()
    );

    match verdict {
        Verdict::Pass if first_row.is_none() && latency_binds => {
            "latency constraints passed".to_string()
        }
        Verdict::Pass if first_row.is_none() => "no assertion enforced".to_string(),
        Verdict::Pass => judged_comparison(first_row),
        Verdict::Fail if is_budget_exhausted => "budget exhausted".to_string(),
        Verdict::Fail if single_row_trigger || triggering.is_empty() => {
            judged_comparison(first_row)
        }
        Verdict::Fail => format!("failed: {}", triggered_by(binding_rows, triggering)),
        Verdict::Inconclusive => {
            if !covariate_status.aligned() {
                "covariate misalignment".to_string()
            } else if is_budget_exhausted {
                "budget exhausted".to_string()
            } else if triggering.is_empty() {
                "insufficient evidence".to_string()
            } else {
                format!("inconclusive: {}", triggered_by(binding_rows, triggering))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::latency::resolver::ThresholdProvenance;
    use crate::latency::{EvaluationStatus, Percentile};
    use crate::model::{CostSummary, TerminationInfo, TerminationReason};
    use crate::statistics::decision::DimensionDecisions;
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
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
        )
        .build();

        assert_eq!(record.verdict(), Some(Verdict::Pass));
        assert_eq!(record.methodology_version(), "1.6.0");
        assert_eq!(record.configuration_errors().len(), 0);
        assert_eq!(record.intent(), TestIntent::Verification);
        assert_eq!(record.identity().service_contract_id(), "shopping-basket");
        assert!(record.statistical_analysis().is_none());
        assert!(record.spec_provenance().is_none());
        assert_eq!(record.warnings().len(), 0);
    }

    #[test]
    fn builds_full_verdict_record() {
        let analysis = crate::oracle_examples::analysis_of(
            &crate::oracle_examples::regression_row("worked_example_pass_at_cutoff", "result"),
        );

        let provenance = SpecProvenance::new(ThresholdOrigin::Empirical)
            .with_spec_filename("shopping-basket.yaml")
            .with_contract_ref("Baseline v1");

        let record = VerdictRecord::builder(
            TestIdentity::new("shopping-basket").with_test_name("test_translation"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalAssessment::single(CriterionRow::result(
                95,
                5,
                vec![("parse".to_string(), 3), ("content".to_string(), 2)],
                Verdict::Pass,
            )),
        )
        .statistical_analysis(analysis)
        .spec_provenance(provenance)
        .warning(Warning::new("BASELINE_EXPIRED", "Baseline is 45 days old"))
        .build();

        assert!(record.statistical_analysis().is_some());
        let stats = record.statistical_analysis().unwrap();
        assert!((stats.threshold() - 0.91).abs() < 1e-10);
        assert_eq!(stats.decision_rule(), DecisionRule::RegressionFisher);

        assert!(record.spec_provenance().is_some());
        let prov = record.spec_provenance().unwrap();
        assert_eq!(prov.spec_filename(), Some("shopping-basket.yaml"));

        assert_eq!(record.warnings().len(), 1);
    }

    // --- Verdict reason derivation ---

    #[test]
    fn verdict_reason_pass() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Pass,
            TestIntent::Verification,
            sample_execution(),
            FunctionalAssessment::single(crate::oracle_examples::regression_row(
                "worked_example_pass_at_cutoff",
                "result",
            )),
        )
        .build();

        assert_eq!(
            record.verdict_reason(),
            "91 of 100 >= cutoff 91 (regression/fisher)"
        );
    }

    #[test]
    fn verdict_reason_fail_completed() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution(),
            FunctionalAssessment::single(crate::oracle_examples::compliance_row(
                "p95_n150_fail_below_k_min",
                "result",
                ThresholdOrigin::Sla,
            )),
        )
        .build();

        assert_eq!(
            record.verdict_reason(),
            "147 of 150 < k_min 148 (compliance/exact-binomial)"
        );
    }

    #[test]
    fn verdict_reason_names_the_triggering_criteria() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution(),
            FunctionalAssessment::single(CriterionRow::result(80, 20, vec![], Verdict::Fail)),
        )
        .triggering(
            crate::statistics::decision::compose_overall_verdict(
                DimensionDecisions {
                    decisions: &[("result".to_owned(), Verdict::Fail)],
                    mode: EnforcementMode::Enforced,
                },
                DimensionDecisions {
                    decisions: &[("p95".to_owned(), Verdict::Fail)],
                    mode: EnforcementMode::Enforced,
                },
            )
            .triggering()
            .to_vec(),
        )
        .build();

        assert_eq!(
            record.verdict_reason(),
            "failed: result: 80 of 100 observed; latency p95"
        );
    }

    #[test]
    fn a_refused_record_has_no_verdict_and_names_every_code_in_order() {
        let record = VerdictRecord::refused(
            TestIdentity::new("test"),
            TestIntent::Verification,
            sample_execution(),
            vec![
                ConfigurationError::ComplianceInfeasible,
                ConfigurationError::TestLargerThanBaseline,
            ],
        )
        .build();
        assert_eq!(record.verdict(), None);
        assert!(record.is_refused());
        assert!(!record.passed());
        assert_eq!(
            record.verdict_reason(),
            "configuration refused: TEST_LARGER_THAN_BASELINE COMPLIANCE_INFEASIBLE"
        );
        assert!(record.functional_assessment().criteria().is_empty());
    }

    #[test]
    fn envelopes_sum_alpha_by_direction_over_the_decisions() {
        let record = VerdictRecord::builder(
            TestIdentity::new("test"),
            Verdict::Fail,
            TestIntent::Verification,
            sample_execution(),
            FunctionalAssessment::new(
                crate::oracle_examples::two_criteria("two_criteria_fail_regression").0,
            ),
        )
        .build();
        // Oracle case `two_criteria_fail_regression`: compliance at alpha
        // 0.01, regression at alpha 0.05.
        let envelopes = record.envelopes();
        assert!((envelopes.false_compliance().unwrap() - 0.01).abs() < 1e-12);
        assert!((envelopes.false_degradation_signal().unwrap() - 0.05).abs() < 1e-12);
        assert_eq!(record.single_decision_rule(), None);
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
            FunctionalAssessment::single(CriterionRow::result(30, 20, vec![], Verdict::Fail)),
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
            FunctionalAssessment::single(CriterionRow::result(
                85,
                15,
                vec![],
                Verdict::Inconclusive,
            )),
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
            FunctionalAssessment::single(CriterionRow::result(
                15,
                5,
                vec![],
                Verdict::Inconclusive,
            )),
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
            FunctionalAssessment::single(CriterionRow::result(7, 3, vec![], Verdict::Inconclusive)),
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
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
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
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
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
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
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
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
        )
        .build();

        assert!(record.correlation_id().is_none());
        assert!(record.pacing().is_none());
        assert_eq!(record.environment().len(), 0);
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
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
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

    /// A record whose functional dimension FAILs and whose explicit p95
    /// latency requirement FAILs, under the given modes; its verdict is the
    /// composite of the enforced dimensions.
    fn both_failing(functional: EnforcementMode, latency: EnforcementMode) -> VerdictRecord {
        let evaluation = crate::latency::LatencyEvaluation::new(
            Percentile::P95,
            Some(Duration::from_millis(600)),
            Some(Duration::from_millis(500)),
            ThresholdProvenance::Explicit,
            EvaluationStatus::Fail,
        );
        let dimension = LatencyDimension::from_parts(vec![evaluation], 90).with_mode(latency);
        let overall = crate::statistics::decision::compose_overall_verdict(
            DimensionDecisions {
                decisions: &[("result".to_owned(), Verdict::Fail)],
                mode: functional,
            },
            DimensionDecisions {
                decisions: &dimension.constraint_verdicts(),
                mode: latency,
            },
        );
        VerdictRecord::builder(
            TestIdentity::new("test"),
            overall.verdict(),
            TestIntent::Verification,
            sample_execution(),
            FunctionalAssessment::single(CriterionRow::result(80, 20, vec![], Verdict::Fail))
                .with_mode(functional),
        )
        .triggering(overall.triggering().to_vec())
        .latency(dimension)
        .build()
    }

    const ENFORCED: EnforcementMode = EnforcementMode::Enforced;
    const ADVISORY: EnforcementMode = EnforcementMode::Advisory;

    fn panics(assertion: impl FnOnce() + std::panic::UnwindSafe) -> Option<String> {
        std::panic::catch_unwind(assertion).err().map(|payload| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default()
        })
    }

    #[test]
    fn with_every_dimension_enforced_each_assertion_fails_on_its_dimension() {
        let record = both_failing(ENFORCED, ENFORCED);
        assert_eq!(record.verdict(), Some(Verdict::Fail));
        assert!(panics(|| record.assert_contract()).is_some());
        assert!(panics(|| record.assert_latency()).is_some());
        let message = panics(|| record.assert_all()).unwrap();
        assert!(message.contains("functional contract failed"));
        assert!(message.contains("latency contract failed (not passed: p95)"));
        assert!(!message.contains("advisory"));
    }

    #[test]
    fn an_advisory_functional_dimension_never_fails_an_assertion() {
        let record = both_failing(ADVISORY, ENFORCED);
        assert_eq!(record.verdict(), Some(Verdict::Fail));
        assert_eq!(panics(|| record.assert_contract()), None);
        let message = panics(|| record.assert_all()).unwrap();
        assert!(!message.contains("functional contract failed"));
        assert!(message.contains("advisory (does not fail the test): functional verdict = FAIL"));
        assert_eq!(record.triggering().len(), 1);
        assert_eq!(record.triggering()[0].kind(), TriggerKind::Latency);
    }

    #[test]
    fn an_advisory_latency_dimension_never_fails_an_assertion() {
        let record = both_failing(ENFORCED, ADVISORY);
        assert_eq!(panics(|| record.assert_latency()), None);
        let message = panics(|| record.assert_all()).unwrap();
        assert!(message.contains("functional contract failed"));
        assert!(message.contains("advisory (does not fail the test): latency verdict = FAIL"));
        assert_eq!(
            record.verdict_reason(),
            "80 of 100 observed; latency advisory"
        );
    }

    #[test]
    fn with_both_dimensions_advisory_the_test_passes_and_reports_both() {
        let record = both_failing(ADVISORY, ADVISORY);
        assert_eq!(record.verdict(), Some(Verdict::Pass));
        assert!(record.passed());
        assert_eq!(panics(|| record.assert_all()), None);
        assert_eq!(record.latency().unwrap().verdict(), Some(Verdict::Fail));
        assert_eq!(record.functional_assessment().composite(), Verdict::Fail);
        assert_eq!(
            record.verdict_reason(),
            "no assertion enforced; functional and latency advisory"
        );
    }

    #[test]
    fn an_advisory_dimension_adds_nothing_to_the_envelopes_or_the_rule() {
        let enforced = both_failing(ENFORCED, ENFORCED);
        assert_eq!(
            enforced.single_decision_rule(),
            Some(DecisionRule::LatencyComplianceExactBinomial)
        );
        assert!(enforced.envelopes().false_compliance().is_some());
        let advisory = both_failing(ENFORCED, ADVISORY);
        assert_eq!(advisory.single_decision_rule(), None);
        assert_eq!(advisory.envelopes().false_compliance(), None);
    }

    #[test]
    fn a_refused_record_fails_every_assertion_whatever_the_modes() {
        let record = VerdictRecord::refused(
            TestIdentity::new("test"),
            TestIntent::Verification,
            sample_execution(),
            vec![ConfigurationError::ComplianceInfeasible],
        )
        .build();
        for assertion in [
            VerdictRecord::assert_contract,
            VerdictRecord::assert_latency,
            VerdictRecord::assert_all,
        ] {
            let message = panics(|| assertion(&record)).unwrap();
            assert!(message.contains("configuration refused before any sample ran"));
        }
    }
}
