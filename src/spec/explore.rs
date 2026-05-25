//! Exploration YAML output: per-configuration specs with descriptive statistics.
//!
//! Each explored configuration produces its own YAML file containing aggregate
//! statistics (observed pass rate, successes, failures) and optional per-sample
//! result projections. Exploration output is descriptive, not inferential — no
//! standard error, confidence intervals, or derived thresholds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::experiment::{ExecutionResult, ExploreResult};
use crate::spec::baseline::{CostBlock, ExecutionBlock};
use crate::spec::common::{build_cost_block, build_failure_distribution, now_iso8601, round4};
use crate::spec::projection::format_projections;

/// A serde-friendly wrapper for factor values in YAML output.
///
/// Serializes values as their natural YAML type (string, number, boolean)
/// without enum tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FactorYamlValue {
    /// A string factor value.
    String(String),
    /// A floating-point factor value.
    Float(f64),
    /// An integer factor value.
    Int(i64),
    /// A boolean factor value.
    Bool(bool),
}

/// An exploration spec produced for a single configuration.
///
/// Uses descriptive statistics only — no inferential statistics (standard
/// error, confidence intervals) because exploration sample counts are too
/// small (typically 1-10) for meaningful inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// javai-ref: JVI-8CHB31R — do not remove (resolves in javai-orchestrator)
pub struct ExplorationSpec {
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

    /// Factor values that define this configuration.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub execution_context: BTreeMap<String, FactorYamlValue>,

    /// Execution details.
    pub execution: ExecutionBlock,

    /// Descriptive statistics.
    pub statistics: ExplorationStatisticsBlock,

    /// Cost summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostBlock>,
}

impl ExplorationSpec {
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

/// Descriptive statistics for an exploration configuration.
///
/// Unlike the baseline [`StatisticsBlock`](crate::spec::baseline::StatisticsBlock),
/// this block is flat and descriptive — no standard error, confidence intervals,
/// or nested success rate block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationStatisticsBlock {
    /// Observed pass rate (successes / total).
    pub observed: f64,

    /// Number of successful trials.
    pub successes: u32,

    /// Number of failed trials.
    pub failures: u32,

    /// Distribution of failures by postcondition check name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_distribution: Option<BTreeMap<String, u32>>,
}

/// Writes per-configuration exploration specs to disk.
// javai-ref: JVI-8CHB31R — do not remove (resolves in javai-orchestrator)
pub struct ExploreSpecWriter {
    output_dir: PathBuf,
}

impl ExploreSpecWriter {
    /// Creates a new writer that writes to the given output directory.
    #[must_use]
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            output_dir: output_dir.into(),
        }
    }

    /// Writes per-configuration specs for all configs in the explore result.
    ///
    /// Returns the paths of all written files.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or file writing fails.
    pub fn write_all(
        &self,
        result: &ExploreResult,
        factor_values: &BTreeMap<String, BTreeMap<String, FactorYamlValue>>,
    ) -> Result<Vec<PathBuf>, std::io::Error> {
        let dir = self.output_dir.join(result.service_contract_id());

        std::fs::create_dir_all(&dir)?;
        let dir = dir.canonicalize()?;

        let mut paths = Vec::new();
        for config in result.configs() {
            let spec = Self::build_spec(
                result.service_contract_id(),
                result.experiment_id(),
                config.execution(),
                factor_values.get(config.name()),
            );
            let path = dir.join(format!("{}.yaml", config.name()));
            let mut yaml = spec.to_yaml().map_err(std::io::Error::other)?;

            let projection_yaml = format_projections(config.projections());
            if !projection_yaml.is_empty() {
                yaml.push_str(&projection_yaml);
            }

            std::fs::write(&path, yaml)?;
            paths.push(path);
        }

        Ok(paths)
    }

    /// Writes a single exploration spec for a named configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or file writing fails.
    pub fn write_one(
        &self,
        service_contract_id: &str,
        experiment_id: Option<&str>,
        config_name: &str,
        execution: &ExecutionResult,
        factors: Option<&BTreeMap<String, FactorYamlValue>>,
    ) -> Result<PathBuf, std::io::Error> {
        let dir = self.output_dir.join(service_contract_id);
        std::fs::create_dir_all(&dir)?;

        let spec = Self::build_spec(service_contract_id, experiment_id, execution, factors);
        let path = dir.join(format!("{config_name}.yaml"));
        let yaml = spec.to_yaml().map_err(std::io::Error::other)?;
        std::fs::write(&path, yaml)?;
        Ok(path)
    }

    fn build_spec(
        service_contract_id: &str,
        experiment_id: Option<&str>,
        execution: &ExecutionResult,
        factors: Option<&BTreeMap<String, FactorYamlValue>>,
    ) -> ExplorationSpec {
        let summary = execution.summary();
        let agg = execution.aggregate();

        ExplorationSpec {
            schema_version: "feotest-spec-1".to_owned(),
            service_contract_id: service_contract_id.to_owned(),
            generated_at: now_iso8601(),
            experiment_id: experiment_id.map(str::to_owned),
            execution_context: factors.cloned().unwrap_or_default(),
            execution: ExecutionBlock {
                samples_planned: summary.samples_planned(),
                samples_executed: summary.samples_executed(),
                termination_reason: Some(summary.termination().reason().to_string()),
            },
            statistics: ExplorationStatisticsBlock {
                observed: round4(summary.observed_pass_rate()),
                successes: summary.successes(),
                failures: summary.failures(),
                failure_distribution: build_failure_distribution(agg),
            },
            cost: Some(build_cost_block(summary.cost())),
        }
    }

    /// The output directory for exploration specs.
    #[must_use]
    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exploration_spec_round_trips_yaml() {
        let spec = ExplorationSpec {
            schema_version: "feotest-spec-1".to_owned(),
            service_contract_id: "shopping-basket".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: Some("model-comparison".to_owned()),
            execution_context: BTreeMap::from([
                (
                    "model".to_owned(),
                    FactorYamlValue::String("gpt-4".to_owned()),
                ),
                ("temperature".to_owned(), FactorYamlValue::Float(0.7)),
            ]),
            execution: ExecutionBlock {
                samples_planned: 5,
                samples_executed: 5,
                termination_reason: Some("COMPLETED".to_owned()),
            },
            statistics: ExplorationStatisticsBlock {
                observed: 0.8,
                successes: 4,
                failures: 1,
                failure_distribution: Some(BTreeMap::from([("relevance-check".to_owned(), 1)])),
            },
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        let restored = ExplorationSpec::from_yaml(&yaml).unwrap();

        assert_eq!(restored.schema_version, "feotest-spec-1");
        assert_eq!(restored.service_contract_id, "shopping-basket");
        assert_eq!(restored.statistics.successes, 4);
        assert_eq!(restored.execution_context.len(), 2);
    }

    #[test]
    fn yaml_uses_camel_case() {
        let spec = ExplorationSpec {
            schema_version: "feotest-spec-1".to_owned(),
            service_contract_id: "test".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: None,
            execution_context: BTreeMap::new(),
            execution: ExecutionBlock {
                samples_planned: 5,
                samples_executed: 5,
                termination_reason: None,
            },
            statistics: ExplorationStatisticsBlock {
                observed: 1.0,
                successes: 5,
                failures: 0,
                failure_distribution: None,
            },
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(yaml.contains("schemaVersion"));
        assert!(yaml.contains("useCaseId"));
        assert!(yaml.contains("samplesPlanned"));
    }

    #[test]
    fn empty_execution_context_omitted() {
        let spec = ExplorationSpec {
            schema_version: "feotest-spec-1".to_owned(),
            service_contract_id: "test".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: None,
            execution_context: BTreeMap::new(),
            execution: ExecutionBlock {
                samples_planned: 5,
                samples_executed: 5,
                termination_reason: None,
            },
            statistics: ExplorationStatisticsBlock {
                observed: 1.0,
                successes: 5,
                failures: 0,
                failure_distribution: None,
            },
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(!yaml.contains("executionContext"));
    }

    #[test]
    fn factor_values_serialize_as_natural_types() {
        let spec = ExplorationSpec {
            schema_version: "feotest-spec-1".to_owned(),
            service_contract_id: "test".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: None,
            execution_context: BTreeMap::from([
                (
                    "model".to_owned(),
                    FactorYamlValue::String("gpt-4".to_owned()),
                ),
                ("temperature".to_owned(), FactorYamlValue::Float(0.7)),
                ("maxTokens".to_owned(), FactorYamlValue::Int(1000)),
                ("streaming".to_owned(), FactorYamlValue::Bool(true)),
            ]),
            execution: ExecutionBlock {
                samples_planned: 5,
                samples_executed: 5,
                termination_reason: None,
            },
            statistics: ExplorationStatisticsBlock {
                observed: 1.0,
                successes: 5,
                failures: 0,
                failure_distribution: None,
            },
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        // String values are quoted, numbers and booleans are not
        assert!(yaml.contains("model: gpt-4"));
        assert!(yaml.contains("temperature: 0.7"));
        assert!(yaml.contains("maxTokens: 1000"));
        assert!(yaml.contains("streaming: true"));
    }

    #[test]
    fn descriptive_statistics_has_no_confidence_interval() {
        let spec = ExplorationSpec {
            schema_version: "feotest-spec-1".to_owned(),
            service_contract_id: "test".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: None,
            execution_context: BTreeMap::new(),
            execution: ExecutionBlock {
                samples_planned: 5,
                samples_executed: 5,
                termination_reason: None,
            },
            statistics: ExplorationStatisticsBlock {
                observed: 0.8,
                successes: 4,
                failures: 1,
                failure_distribution: None,
            },
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(!yaml.contains("standardError"));
        assert!(!yaml.contains("confidenceInterval"));
        assert!(!yaml.contains("minPassRate"));
    }
}
