//! Baseline spec: the YAML-serializable measurement result.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A baseline specification produced by a measure experiment.
///
/// Contains all data needed to derive a threshold for probabilistic testing:
/// observed success rate, confidence interval, sample size, and metadata.
///
/// Serialized to YAML as the `feotest-spec-1` schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// mavai-ref: JVI-EC8CPT3 — do not remove (resolves in mavai-orchestrator)
pub struct BaselineSpec {
    /// Schema version identifier.
    pub schema_version: String,

    /// The service contract identifier.
    #[serde(rename = "useCaseId")]
    pub service_contract_id: String,

    /// ISO 8601 timestamp of when the spec was generated.
    pub generated_at: String,

    /// The experiment that produced this spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,

    /// Invocation footprint: 8-char hex hash of service contract ID + covariate
    /// declarations. Identifies *what* covariates are declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,

    /// Resolved covariate values at experiment time.
    /// Keys in declaration order, values as canonical strings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub covariates: BTreeMap<String, String>,

    /// Execution details.
    pub execution: ExecutionBlock,

    /// Derived requirements.
    pub requirements: RequirementsBlock,

    /// Statistical summary.
    pub statistics: StatisticsBlock,

    /// Cost summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostBlock>,

    /// Baseline validity window.
    ///
    /// Present only when the measure experiment was configured with a
    /// non-zero `expiresInDays`. Absent means no expiration policy and
    /// no expiration checks will be performed at test time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiration: Option<ExpirationBlock>,

    /// Integrity hash of the spec content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_fingerprint: Option<String>,
}

/// Execution details within a baseline spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionBlock {
    /// Number of samples originally planned.
    pub samples_planned: u32,

    /// Number of samples actually executed.
    pub samples_executed: u32,

    /// Why execution terminated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination_reason: Option<String>,
}

/// Derived requirements within a baseline spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementsBlock {
    /// Minimum pass rate derived from the measurement.
    ///
    /// Typically the lower bound of the 95% Wilson score confidence interval.
    pub min_pass_rate: f64,
}

/// Statistical summary within a baseline spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatisticsBlock {
    /// Success rate statistics.
    pub success_rate: SuccessRateBlock,

    /// Raw success count.
    pub successes: u32,

    /// Raw failure count.
    pub failures: u32,

    /// Distribution of failures by postcondition check name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_distribution: Option<std::collections::BTreeMap<String, u32>>,

    /// Post-warmup successful-response latency distribution.
    ///
    /// Absent for baselines generated before latency capture existed or for
    /// runs that produced no successful trials.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "latency")]
    pub latency_distribution: Option<LatencyBlock>,

    /// Per-criterion success-rate statistics, keyed by criterion name.
    ///
    /// Present when the measured contract declared more than one criterion (or
    /// declared one by name): each `empirical()` criterion resolves *its own*
    /// target from its entry here. The aggregate [`success_rate`] above remains
    /// the whole-contract figure. Absent for single, unnamed-criterion
    /// baselines and for baselines generated before per-criterion measurement
    /// existed — those resolve every empirical criterion against the aggregate.
    ///
    /// [`success_rate`]: Self::success_rate
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_criterion: Option<std::collections::BTreeMap<String, CriterionStatistics>>,
}

/// Per-criterion success-rate statistics within a baseline spec.
///
/// Mirrors the whole-contract figures ([`StatisticsBlock`]) for a single
/// named criterion, so an `empirical()` criterion can derive its target from
/// the rate observed for *that* criterion during measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CriterionStatistics {
    /// Success rate statistics for this criterion.
    pub success_rate: SuccessRateBlock,

    /// Raw success count for this criterion.
    pub successes: u32,

    /// Raw failure count for this criterion.
    pub failures: u32,

    /// Distribution of this criterion's failures by postcondition check name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_distribution: Option<std::collections::BTreeMap<String, u32>>,

    /// Normative judgement rendered at measure time.
    ///
    /// Present only for criteria the measured contract declared with a
    /// stipulated pass rate (a normative criterion); absent for empirical
    /// criteria and for baselines generated before judgement recording
    /// existed. Threshold derivation and spec resolution ignore this block —
    /// it is a durable record for later readers of the file, not an input to
    /// any computation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normative_judgement: Option<NormativeJudgementBlock>,
}

/// The recorded state of a normative judgement within a baseline spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NormativeJudgementState {
    /// The run's evidence cleared the stipulated threshold.
    Met,
    /// The run's evidence did not clear the stipulated threshold.
    Failed,
    /// The run's sample count could not support the stipulated threshold at
    /// the judgement confidence, even with a perfect observation.
    Unsupportable,
}

/// Normative judgement block within a baseline spec.
///
/// Records the relation of a measure run's evidence to a stipulation in
/// force at measure time: the judgement state, the stipulated threshold it
/// was judged against, and the confidence of the judgement. It states
/// nothing further about the service under test — a failed judgement at
/// measure time can be entirely expected (an aspirational bar measured
/// mid-development, a fresh configuration characterised before tuning).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormativeJudgementBlock {
    /// The judgement state.
    pub state: NormativeJudgementState,

    /// The stipulated threshold the run was judged against.
    pub stipulated_threshold: f64,

    /// The confidence level of the judgement.
    pub confidence: f64,

    /// The smallest sample count at which a perfect observation would clear
    /// the stipulated threshold. Present only when `state` is
    /// [`NormativeJudgementState::Unsupportable`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feasible_minimum_samples: Option<u32>,
}

/// Latency block within a baseline spec.
///
/// Stores the full sorted vector of successful-response latencies so that
/// thresholds can be re-resolved exactly at verdict time for any chosen
/// `(percentile, confidence)` pair. This matches the non-parametric
/// derivation in `mavai-R/R/latency.R::latency_threshold_derive`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LatencyBlock {
    /// Post-warmup successful-response latencies in milliseconds, sorted
    /// ascending.
    pub latencies_ms: Vec<u64>,

    /// Sample mean in milliseconds, rounded.
    pub mean_ms: u64,

    /// Observed maximum in milliseconds.
    pub max_ms: u64,
}

/// Success rate statistics within a baseline spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessRateBlock {
    /// Observed success rate.
    pub observed: f64,

    /// Standard error of the observed rate.
    pub standard_error: f64,

    /// 95% confidence interval as [lower, upper].
    pub confidence_interval95: [f64; 2],
}

/// Baseline validity window within a baseline spec.
///
/// Records how long the measurement remains representative of the service
/// under test. At test time, the [`crate::spec::expiration`] evaluator
/// compares the current time against `expiration_date` to decide whether
/// the baseline is still fresh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpirationBlock {
    /// Validity window in days. Must be non-zero; a zero-day policy is
    /// represented by omitting the whole block.
    pub expires_in_days: u32,

    /// ISO 8601 timestamp of when the measurement run ended.
    pub baseline_end_time: String,

    /// Derived ISO 8601 timestamp of when the baseline becomes stale.
    ///
    /// Written for human readability — the evaluator recomputes this
    /// value from `baseline_end_time + expires_in_days` at check time.
    pub expiration_date: String,
}

/// Cost summary within a baseline spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostBlock {
    /// Total wall-clock time in milliseconds.
    pub total_time_ms: u64,

    /// Average time per sample in milliseconds.
    pub avg_time_per_sample_ms: u64,

    /// Total tokens consumed.
    pub total_tokens: u64,

    /// Average tokens per sample.
    pub avg_tokens_per_sample: u64,
}

/// Errors that can occur when loading a baseline spec from YAML.
#[derive(Debug)]
pub enum SpecLoadError {
    /// The YAML could not be parsed.
    Parse(serde_yaml::Error),
    /// The content fingerprint is missing.
    MissingFingerprint {
        /// The service contract ID of the spec.
        service_contract_id: String,
    },
    /// The content fingerprint does not match the spec content.
    IntegrityFailure {
        /// The service contract ID of the spec.
        service_contract_id: String,
        /// The fingerprint stored in the spec.
        expected: String,
        /// The fingerprint recomputed from the content.
        actual: String,
    },
}

impl std::fmt::Display for SpecLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "failed to parse baseline spec: {e}"),
            Self::MissingFingerprint {
                service_contract_id,
            } => write!(
                f,
                "baseline spec for '{service_contract_id}' has no contentFingerprint — \
                 re-run the measure experiment to generate a verified baseline"
            ),
            Self::IntegrityFailure {
                service_contract_id,
                expected,
                actual,
            } => write!(
                f,
                "baseline spec for '{service_contract_id}' has been modified since generation \
                 (expected fingerprint {expected}, computed {actual}) — \
                 re-run the measure experiment to generate a fresh baseline"
            ),
        }
    }
}

impl std::error::Error for SpecLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(e) => Some(e),
            Self::MissingFingerprint { .. } | Self::IntegrityFailure { .. } => None,
        }
    }
}

impl BaselineSpec {
    /// The schema version for the current format.
    pub const SCHEMA_VERSION: &'static str = "feotest-spec-1";

    /// Creates a new baseline spec with required fields.
    #[must_use]
    pub fn new(
        service_contract_id: impl Into<String>,
        generated_at: impl Into<String>,
        execution: ExecutionBlock,
        requirements: RequirementsBlock,
        statistics: StatisticsBlock,
    ) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            service_contract_id: service_contract_id.into(),
            generated_at: generated_at.into(),
            experiment_id: None,
            footprint: None,
            covariates: BTreeMap::new(),
            execution,
            requirements,
            statistics,
            cost: None,
            expiration: None,
            content_fingerprint: None,
        }
    }

    /// Serializes the spec to YAML.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Deserializes a spec from YAML, verifying content integrity.
    ///
    /// Recomputes the SHA-256 fingerprint and compares it to the stored
    /// `contentFingerprint` value. If the fingerprint is missing or does
    /// not match, the spec is rejected.
    ///
    /// # Errors
    ///
    /// Returns an error if the YAML is malformed, the fingerprint is
    /// missing, or the fingerprint does not match the content.
    pub fn from_yaml(yaml: &str) -> Result<Self, SpecLoadError> {
        let spec: Self = serde_yaml::from_str(yaml).map_err(SpecLoadError::Parse)?;
        verify_integrity(yaml, &spec)?;
        Ok(spec)
    }

    /// Deserializes a spec from YAML without verifying integrity.
    ///
    /// # Errors
    ///
    /// Returns an error if the YAML is malformed.
    #[cfg(test)]
    pub(crate) fn parse_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
}

/// Verifies the integrity of a baseline spec against its content fingerprint.
// mavai-ref: JVI-CNDHE1$ — do not remove (resolves in mavai-orchestrator)
fn verify_integrity(yaml: &str, spec: &BaselineSpec) -> Result<(), SpecLoadError> {
    let stored =
        spec.content_fingerprint
            .as_ref()
            .ok_or_else(|| SpecLoadError::MissingFingerprint {
                service_contract_id: spec.service_contract_id.clone(),
            })?;

    let hashable = content_before_fingerprint(yaml);
    let digest = Sha256::digest(hashable.as_bytes());
    let computed = format!("{digest:x}");

    if computed != *stored {
        return Err(SpecLoadError::IntegrityFailure {
            service_contract_id: spec.service_contract_id.clone(),
            expected: stored.clone(),
            actual: computed,
        });
    }
    Ok(())
}

/// Extracts the YAML content before the `contentFingerprint:` line.
///
/// The write-side algorithm serializes the spec with `content_fingerprint = None`
/// (which omits the field entirely), hashes that YAML string, then re-serializes
/// with the fingerprint included. Because `contentFingerprint` is the last field
/// in the struct, the YAML content before the `contentFingerprint:` line is
/// exactly the string that was hashed at write time.
fn content_before_fingerprint(yaml: &str) -> &str {
    yaml.find("\ncontentFingerprint:")
        .map_or(yaml, |pos| &yaml[..=pos])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> BaselineSpec {
        BaselineSpec::new(
            "shopping-basket",
            "2026-03-27T10:00:00Z",
            ExecutionBlock {
                samples_planned: 1000,
                samples_executed: 1000,
                termination_reason: Some("COMPLETED".to_string()),
            },
            RequirementsBlock {
                min_pass_rate: 0.7512,
            },
            StatisticsBlock {
                success_rate: SuccessRateBlock {
                    observed: 0.777,
                    standard_error: 0.0132,
                    confidence_interval95: [0.7512, 0.8028],
                },
                successes: 777,
                failures: 223,
                failure_distribution: None,
                latency_distribution: None,
                per_criterion: None,
            },
        )
    }

    #[test]
    fn round_trips_through_yaml() {
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();
        let restored = BaselineSpec::parse_yaml(&yaml).unwrap();

        assert_eq!(restored.schema_version, BaselineSpec::SCHEMA_VERSION);
        assert_eq!(restored.service_contract_id, "shopping-basket");
        assert_eq!(restored.execution.samples_executed, 1000);
        assert!((restored.requirements.min_pass_rate - 0.7512).abs() < 1e-10);
        assert_eq!(restored.statistics.successes, 777);
    }

    #[test]
    fn per_criterion_statistics_round_trip() {
        let mut spec = sample_spec();
        let mut per_criterion = BTreeMap::new();
        per_criterion.insert(
            "non-empty".to_string(),
            CriterionStatistics {
                success_rate: SuccessRateBlock {
                    observed: 0.95,
                    standard_error: 0.0069,
                    confidence_interval95: [0.9364, 0.9636],
                },
                successes: 950,
                failures: 50,
                failure_distribution: None,
                normative_judgement: None,
            },
        );
        spec.statistics.per_criterion = Some(per_criterion);

        let yaml = spec.to_yaml().unwrap();
        assert!(yaml.contains("perCriterion"));

        let restored = BaselineSpec::parse_yaml(&yaml).unwrap();
        let criterion = &restored.statistics.per_criterion.unwrap()["non-empty"];
        assert_eq!(criterion.successes, 950);
        assert_eq!(criterion.failures, 50);
        assert!((criterion.success_rate.observed - 0.95).abs() < 1e-10);
    }

    #[test]
    fn normative_judgement_round_trips_through_yaml() {
        let mut spec = sample_spec();
        let mut per_criterion = BTreeMap::new();
        per_criterion.insert(
            "transaction succeeds".to_string(),
            CriterionStatistics {
                success_rate: SuccessRateBlock {
                    observed: 0.983,
                    standard_error: 0.0041,
                    confidence_interval95: [0.9741, 0.9891],
                },
                successes: 983,
                failures: 17,
                failure_distribution: None,
                normative_judgement: Some(NormativeJudgementBlock {
                    state: NormativeJudgementState::Failed,
                    stipulated_threshold: 0.99,
                    confidence: 0.95,
                    feasible_minimum_samples: None,
                }),
            },
        );
        spec.statistics.per_criterion = Some(per_criterion);

        let yaml = spec.to_yaml().unwrap();
        assert!(yaml.contains("normativeJudgement:"));
        assert!(yaml.contains("state: failed"));
        assert!(yaml.contains("stipulatedThreshold: 0.99"));
        assert!(yaml.contains("confidence: 0.95"));

        let restored = BaselineSpec::parse_yaml(&yaml).unwrap();
        let judgement = restored.statistics.per_criterion.unwrap()["transaction succeeds"]
            .normative_judgement
            .clone()
            .unwrap();
        assert_eq!(judgement.state, NormativeJudgementState::Failed);
        assert!((judgement.stipulated_threshold - 0.99).abs() < 1e-10);
        assert!((judgement.confidence - 0.95).abs() < 1e-10);
        assert!(judgement.feasible_minimum_samples.is_none());
    }

    #[test]
    fn unsupportable_judgement_records_feasible_minimum() {
        let block = NormativeJudgementBlock {
            state: NormativeJudgementState::Unsupportable,
            stipulated_threshold: 0.999,
            confidence: 0.95,
            feasible_minimum_samples: Some(2995),
        };
        let yaml = serde_yaml::to_string(&block).unwrap();
        assert!(yaml.contains("state: unsupportable"));
        assert!(yaml.contains("feasibleMinimumSamples: 2995"));

        let restored: NormativeJudgementBlock = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(restored.feasible_minimum_samples, Some(2995));
    }

    #[test]
    fn normative_judgement_absent_by_default_and_pre_existing_specs_parse() {
        // A per-criterion block written before judgement recording existed
        // carries no normativeJudgement key — it must parse unchanged, with
        // the judgement absent.
        let yaml = "\
successRate:
  observed: 0.95
  standardError: 0.0069
  confidenceInterval95: [0.9364, 0.9636]
successes: 950
failures: 50
";
        let restored: CriterionStatistics = serde_yaml::from_str(yaml).unwrap();
        assert!(restored.normative_judgement.is_none());

        // And a freshly written block without a judgement omits the key.
        let out = serde_yaml::to_string(&restored).unwrap();
        assert!(!out.contains("normativeJudgement"));
    }

    #[test]
    fn per_criterion_absent_by_default_and_parses_when_omitted() {
        // A single-criterion baseline carries no per-criterion block, and a
        // spec written without one still parses (backward compatibility).
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();
        assert!(!yaml.contains("perCriterion"));
        let restored = BaselineSpec::parse_yaml(&yaml).unwrap();
        assert!(restored.statistics.per_criterion.is_none());
    }

    #[test]
    fn yaml_output_uses_camel_case() {
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();
        assert!(yaml.contains("schemaVersion"));
        assert!(yaml.contains("useCaseId"));
        assert!(yaml.contains("minPassRate"));
        assert!(yaml.contains("samplesPlanned"));
    }

    #[test]
    fn optional_fields_omitted_when_none() {
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();
        assert!(!yaml.contains("experimentId"));
        assert!(!yaml.contains("contentFingerprint"));
    }

    #[test]
    fn schema_version_is_correct() {
        let spec = sample_spec();
        assert_eq!(spec.schema_version, "feotest-spec-1");
    }

    /// Builds a YAML string with a valid content fingerprint.
    fn yaml_with_fingerprint() -> String {
        let spec = sample_spec();
        let yaml_without_fp = spec.to_yaml().unwrap();
        let digest = Sha256::digest(yaml_without_fp.as_bytes());
        let fingerprint = format!("{digest:x}");

        let mut signed = spec;
        signed.content_fingerprint = Some(fingerprint);
        signed.to_yaml().unwrap()
    }

    #[test]
    fn from_yaml_accepts_valid_fingerprint() {
        let yaml = yaml_with_fingerprint();
        let result = BaselineSpec::from_yaml(&yaml);
        assert!(result.is_ok());
        let spec = result.unwrap();
        assert_eq!(spec.service_contract_id, "shopping-basket");
        assert!(spec.content_fingerprint.is_some());
    }

    #[test]
    fn from_yaml_rejects_missing_fingerprint() {
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();

        let result = BaselineSpec::from_yaml(&yaml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SpecLoadError::MissingFingerprint { .. }),
            "expected MissingFingerprint, got: {err}"
        );
        assert!(err.to_string().contains("shopping-basket"));
    }

    #[test]
    fn from_yaml_rejects_tampered_content() {
        let yaml = yaml_with_fingerprint();
        // Tamper: change the observed pass rate
        let tampered = yaml.replace("observed: 0.777", "observed: 0.999");
        assert_ne!(yaml, tampered);

        let result = BaselineSpec::from_yaml(&tampered);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SpecLoadError::IntegrityFailure { .. }),
            "expected IntegrityFailure, got: {err}"
        );
        assert!(err.to_string().contains("shopping-basket"));
        assert!(err.to_string().contains("modified since generation"));
    }

    #[test]
    fn from_yaml_rejects_tampered_min_pass_rate() {
        let yaml = yaml_with_fingerprint();
        // The adversarial case: lowering minPassRate to make a test pass
        let tampered = yaml.replace("minPassRate: 0.7512", "minPassRate: 0.5000");
        assert_ne!(yaml, tampered);

        let result = BaselineSpec::from_yaml(&tampered);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecLoadError::IntegrityFailure { .. }
        ));
    }

    #[test]
    fn from_yaml_rejects_tampered_fingerprint() {
        let yaml = yaml_with_fingerprint();
        // Replace the fingerprint value with a bogus one
        let tampered = yaml.replace(
            yaml.lines()
                .find(|l| l.starts_with("contentFingerprint:"))
                .unwrap(),
            "contentFingerprint: 0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert_ne!(yaml, tampered);

        let result = BaselineSpec::from_yaml(&tampered);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecLoadError::IntegrityFailure { .. }
        ));
    }

    #[test]
    fn from_yaml_rejects_malformed_yaml() {
        let result = BaselineSpec::from_yaml("not: valid: yaml: [[[");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SpecLoadError::Parse(_)));
    }

    fn sample_spec_with_expiration() -> BaselineSpec {
        let mut spec = sample_spec();
        spec.expiration = Some(ExpirationBlock {
            expires_in_days: 30,
            baseline_end_time: "2026-04-19T10:00:00Z".to_string(),
            expiration_date: "2026-05-19T10:00:00Z".to_string(),
        });
        spec
    }

    #[test]
    fn expiration_block_round_trips_through_yaml() {
        let spec = sample_spec_with_expiration();
        let yaml = spec.to_yaml().unwrap();
        assert!(yaml.contains("expiration:"));
        assert!(yaml.contains("expiresInDays: 30"));
        assert!(yaml.contains("baselineEndTime: 2026-04-19T10:00:00Z"));
        assert!(yaml.contains("expirationDate: 2026-05-19T10:00:00Z"));

        let restored = BaselineSpec::parse_yaml(&yaml).unwrap();
        assert_eq!(restored.expiration, spec.expiration);
    }

    #[test]
    fn expiration_block_omitted_when_none() {
        let yaml = sample_spec().to_yaml().unwrap();
        assert!(!yaml.contains("expiration:"));
        assert!(!yaml.contains("expiresInDays"));
    }

    #[test]
    fn fingerprint_covers_expiration_block() {
        let spec = sample_spec_with_expiration();
        let yaml_without_fp = spec.to_yaml().unwrap();
        let digest = Sha256::digest(yaml_without_fp.as_bytes());
        let fingerprint = format!("{digest:x}");
        let mut signed = spec;
        signed.content_fingerprint = Some(fingerprint);
        let yaml = signed.to_yaml().unwrap();

        // Verifies: the expiration block sits before the contentFingerprint
        // line and so is covered by content_before_fingerprint.
        let loaded = BaselineSpec::from_yaml(&yaml).unwrap();
        assert_eq!(
            loaded.expiration.as_ref().map(|e| e.expires_in_days),
            Some(30)
        );
    }

    #[test]
    fn from_yaml_rejects_tampered_expires_in_days() {
        let spec = sample_spec_with_expiration();
        let yaml_without_fp = spec.to_yaml().unwrap();
        let digest = Sha256::digest(yaml_without_fp.as_bytes());
        let fingerprint = format!("{digest:x}");
        let mut signed = spec;
        signed.content_fingerprint = Some(fingerprint);
        let yaml = signed.to_yaml().unwrap();

        // Adversarial: extend the window to resurrect a baseline that
        // should already have expired.
        let tampered = yaml.replace("expiresInDays: 30", "expiresInDays: 365");
        assert_ne!(yaml, tampered);

        let result = BaselineSpec::from_yaml(&tampered);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecLoadError::IntegrityFailure { .. }
        ));
    }

    #[test]
    fn content_before_fingerprint_extracts_hashable_content() {
        let yaml = "schemaVersion: feotest-spec-1\nuseCaseId: test\ncontentFingerprint: abc123\n";
        let hashable = content_before_fingerprint(yaml);
        assert_eq!(hashable, "schemaVersion: feotest-spec-1\nuseCaseId: test\n");
    }

    #[test]
    fn content_before_fingerprint_returns_all_when_no_fingerprint() {
        let yaml = "schemaVersion: feotest-spec-1\nuseCaseId: test\n";
        let hashable = content_before_fingerprint(yaml);
        assert_eq!(hashable, yaml);
    }

    #[test]
    fn write_and_verify_round_trip() {
        // Simulates the write-side algorithm and verifies the read side accepts it
        let spec = sample_spec();
        let yaml_without_fp = spec.to_yaml().unwrap();
        let digest = Sha256::digest(yaml_without_fp.as_bytes());
        let fingerprint = format!("{digest:x}");

        let mut signed = spec;
        signed.content_fingerprint = Some(fingerprint);
        let yaml_with_fp = signed.to_yaml().unwrap();

        // Verify that the hashable content matches what was hashed
        let hashable = content_before_fingerprint(&yaml_with_fp);
        assert_eq!(hashable, yaml_without_fp);

        // Verify from_yaml accepts it
        let loaded = BaselineSpec::from_yaml(&yaml_with_fp).unwrap();
        assert_eq!(loaded.service_contract_id, "shopping-basket");
    }
}
