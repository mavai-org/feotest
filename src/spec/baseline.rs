//! Baseline spec: the YAML-serializable measurement result.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A baseline specification produced by a measure experiment.
///
/// Contains all data needed to derive a threshold for probabilistic testing:
/// observed success rate, confidence interval, sample size, and metadata.
///
/// Serialized to YAML as the `feotest-spec-1` schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineSpec {
    /// Schema version identifier.
    pub schema_version: String,

    /// The use case identifier.
    pub use_case_id: String,

    /// ISO 8601 timestamp of when the spec was generated.
    pub generated_at: String,

    /// The experiment that produced this spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,

    /// Invocation footprint: 8-char hex hash of use case ID + covariate
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

impl BaselineSpec {
    /// The schema version for the current format.
    pub const SCHEMA_VERSION: &'static str = "feotest-spec-1";

    /// Creates a new baseline spec with required fields.
    #[must_use]
    pub fn new(
        use_case_id: impl Into<String>,
        generated_at: impl Into<String>,
        execution: ExecutionBlock,
        requirements: RequirementsBlock,
        statistics: StatisticsBlock,
    ) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            use_case_id: use_case_id.into(),
            generated_at: generated_at.into(),
            experiment_id: None,
            footprint: None,
            covariates: BTreeMap::new(),
            execution,
            requirements,
            statistics,
            cost: None,
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

    /// Deserializes a spec from YAML.
    ///
    /// # Errors
    ///
    /// Returns an error if the YAML is malformed or missing required fields.
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
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
            },
        )
    }

    #[test]
    fn round_trips_through_yaml() {
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();
        let restored = BaselineSpec::from_yaml(&yaml).unwrap();

        assert_eq!(restored.schema_version, BaselineSpec::SCHEMA_VERSION);
        assert_eq!(restored.use_case_id, "shopping-basket");
        assert_eq!(restored.execution.samples_executed, 1000);
        assert!((restored.requirements.min_pass_rate - 0.7512).abs() < 1e-10);
        assert_eq!(restored.statistics.successes, 777);
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
}
