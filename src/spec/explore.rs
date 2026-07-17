//! Exploration YAML output: per-configuration specs with descriptive statistics.
//!
//! Each explored configuration produces its own YAML file in the family's
//! canonical `mavai-explore-1` interchange format, containing aggregate
//! statistics (observed pass rate, successes, failures), per-criterion
//! tallies, stated latency percentiles, and optional per-sample result
//! projections. Exploration output is descriptive, not inferential — no
//! standard error, confidence intervals, or derived thresholds.
//!
//! Latency percentiles are **stated value-or-absent**: each is emitted only
//! when the recorded passing samples clear that percentile's minimum-sample
//! floor. Consumers render absence; they never compute a replacement.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::experiment::{ContractExecutionResult, ExploreResult};
use crate::spec::baseline::{CostBlock, ExecutionBlock};
use crate::spec::common::{
    FailureDistributionEntry, build_cost_block, build_failure_distribution, build_failure_entries,
    now_iso8601, round4,
};
use crate::spec::keys;
use crate::spec::projection::{SampleProjection, format_projections};

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
    /// Schema version identifier: `mavai-explore-1`.
    pub schema_version: String,

    /// The service contract identifier.
    pub service_contract_id: String,

    /// ISO 8601 timestamp of when the spec was generated.
    pub generated_at: String,

    /// The experiment that produced this spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,

    /// The configuration's display name, as supplied by the factor's
    /// `Display` impl. Carried in the body so consumers never need to parse
    /// it back out of the filename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<String>,

    /// Factor values that define this configuration.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub factors: BTreeMap<String, FactorYamlValue>,

    /// Execution details.
    pub execution: ExecutionBlock,

    /// Descriptive statistics.
    pub statistics: ExplorationStatisticsBlock,

    /// Latency detail — the passing-trial durations plus the stated
    /// percentiles. Absent when no sample passed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<ExplorationLatencyBlock>,

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

    /// Failure attribution as a sequence of bounded entries: each failed
    /// trial counted under its first failing condition, so the entries'
    /// counts sum to `failures`. Absent when no trial failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_distribution: Option<Vec<FailureDistributionEntry>>,

    /// Per-criterion tallies, keyed by criterion name. Present whenever the
    /// run judged named criteria — including the single-criterion case, since
    /// the criterion's *name* is what a comparison view needs and the
    /// aggregate above does not carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<BTreeMap<String, ExplorationCriterionBlock>>,
}

/// One criterion's tally within an exploration configuration.
///
/// Descriptive, like its parent block: observed rate and counts only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationCriterionBlock {
    /// Observed pass rate for this criterion (pass / (pass + fail)).
    pub observed_pass_rate: f64,

    /// Number of trials where this criterion passed.
    pub pass: u32,

    /// Number of trials where this criterion failed.
    pub fail: u32,

    /// This criterion's failures as a sequence of bounded entries, in the
    /// same shape as the run-level distribution. Omitted when it would
    /// merely restate a single-condition criterion's `fail` count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_distribution: Option<Vec<FailureDistributionEntry>>,
}

/// Latency detail for an exploration configuration.
///
/// Carries the raw passing-trial durations, sorted ascending, plus the
/// percentiles **stated** under this crate's minimum-sample gates. An
/// absent percentile means "not stateable at this sample count", never
/// zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorationLatencyBlock {
    /// The population this block describes: always `passing-samples`.
    pub basis: String,

    /// How many samples passed (and so contributed durations).
    pub contributing_samples: u32,

    /// How many samples the run executed in total.
    pub total_samples: u32,

    /// Stated median, when the passing count clears its floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_ms: Option<f64>,

    /// Stated 95th percentile, when the passing count clears its floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<f64>,

    /// Stated 99th percentile, when the passing count clears its floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p99_ms: Option<f64>,

    /// Wall-clock durations of the passing trials, in milliseconds, sorted
    /// ascending. For shape-only consumption.
    pub sorted_passing_latencies_ms: Vec<u64>,
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
                config.name(),
                config.execution(),
                config.projections(),
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
        execution: &ContractExecutionResult,
        projections: &[SampleProjection],
        factors: Option<&BTreeMap<String, FactorYamlValue>>,
    ) -> Result<PathBuf, std::io::Error> {
        let dir = self.output_dir.join(service_contract_id);
        std::fs::create_dir_all(&dir)?;

        let spec = Self::build_spec(
            service_contract_id,
            experiment_id,
            config_name,
            execution,
            projections,
            factors,
        );
        let path = dir.join(format!("{config_name}.yaml"));
        let yaml = spec.to_yaml().map_err(std::io::Error::other)?;
        std::fs::write(&path, yaml)?;
        Ok(path)
    }

    fn build_spec(
        service_contract_id: &str,
        experiment_id: Option<&str>,
        config_name: &str,
        execution: &ContractExecutionResult,
        projections: &[SampleProjection],
        factors: Option<&BTreeMap<String, FactorYamlValue>>,
    ) -> ExplorationSpec {
        let summary = execution.summary();
        let agg = execution.aggregate();

        ExplorationSpec {
            schema_version: "mavai-explore-1".to_owned(),
            service_contract_id: service_contract_id.to_owned(),
            generated_at: now_iso8601(),
            experiment_id: experiment_id.map(str::to_owned),
            configuration: Some(config_name.to_owned()),
            factors: factors.cloned().unwrap_or_default(),
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
                criteria: build_criteria_blocks(execution),
            },
            latency: build_latency_block(projections, summary.samples_executed()),
            cost: Some(build_cost_block(summary.cost())),
        }
    }

    /// The output directory for exploration specs.
    #[must_use]
    pub fn output_dir(&self) -> &Path {
        &self.output_dir
    }
}

/// The per-criterion tallies of a run, keyed by criterion name; `None` when
/// the run judged no named criteria.
pub(crate) fn build_criteria_blocks(
    execution: &ContractExecutionResult,
) -> Option<BTreeMap<String, ExplorationCriterionBlock>> {
    let per_criterion = execution.criteria_counts().per_criterion();
    if per_criterion.is_empty() {
        return None;
    }
    Some(
        per_criterion
            .iter()
            .map(|counts| {
                (
                    keys::bounded_identity(counts.criterion()),
                    ExplorationCriterionBlock {
                        observed_pass_rate: round4(counts.pass_rate().unwrap_or(0.0)),
                        pass: counts.pass(),
                        fail: counts.fail(),
                        failure_distribution: criterion_failure_entries(counts),
                    },
                )
            })
            .collect(),
    )
}

/// The per-criterion failure distribution, omitted where it carries no
/// information beyond the criterion's own tally: a single entry whose
/// condition is the criterion's name merely restates `fail`.
fn criterion_failure_entries(
    counts: &crate::criteria::CriterionCounts,
) -> Option<Vec<FailureDistributionEntry>> {
    let entries = build_failure_entries(
        counts
            .failure_distribution()
            .iter()
            .map(|(check, count)| (check.clone(), *count)),
    )?;
    let restates_the_tally = entries.len() == 1
        && entries[0].condition == counts.criterion()
        && entries[0].count == counts.fail();
    (!restates_the_tally).then_some(entries)
}

/// The latency block for a run: the sorted passing-trial durations plus the
/// percentiles stated under this crate's minimum-sample gates. `None` when no
/// sample passed.
pub(crate) fn build_latency_block(
    projections: &[SampleProjection],
    samples_executed: u32,
) -> Option<ExplorationLatencyBlock> {
    let mut latencies: Vec<u64> = projections
        .iter()
        .filter(|projection| projection.is_success())
        .map(SampleProjection::execution_time_ms)
        .collect();
    if latencies.is_empty() {
        return None;
    }
    latencies.sort_unstable();
    let contributing =
        u32::try_from(latencies.len()).expect("passing-sample count exceeds u32::MAX");
    Some(ExplorationLatencyBlock {
        basis: "passing-samples".to_owned(),
        contributing_samples: contributing,
        total_samples: samples_executed,
        p50_ms: stated_percentile(&latencies, 0.50),
        p95_ms: stated_percentile(&latencies, 0.95),
        p99_ms: stated_percentile(&latencies, 0.99),
        sorted_passing_latencies_ms: latencies,
    })
}

/// The nearest-rank percentile of the recorded passing latencies, stated only
/// when the passing count clears the percentile's minimum-sample floor.
pub(crate) fn stated_percentile(sorted_latencies_ms: &[u64], fraction: f64) -> Option<f64> {
    use crate::statistics::latency::{min_samples_for, nearest_rank_percentile};
    if sorted_latencies_ms.len() < min_samples_for(fraction) as usize {
        return None;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "trial durations in ms are far below 2^52"
    )]
    let values: Vec<f64> = sorted_latencies_ms.iter().map(|&ms| ms as f64).collect();
    Some(nearest_rank_percentile(&values, fraction))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_below_their_sample_floor_are_unstated() {
        assert!(stated_percentile(&[1, 2, 3, 4], 0.50).is_none());
        assert!(stated_percentile(&[1, 2, 3, 4, 5], 0.50).is_some());
        assert!(stated_percentile(&(1..=19).collect::<Vec<u64>>(), 0.95).is_none());
        assert!(stated_percentile(&(1..=20).collect::<Vec<u64>>(), 0.95).is_some());
        assert!(stated_percentile(&(1..=99).collect::<Vec<u64>>(), 0.99).is_none());
        assert!(stated_percentile(&(1..=100).collect::<Vec<u64>>(), 0.99).is_some());
    }

    #[test]
    fn stated_percentiles_are_nearest_rank_values() {
        let latencies: Vec<u64> = (1..=100).collect();
        assert!((stated_percentile(&latencies, 0.50).unwrap() - 50.0).abs() < f64::EPSILON);
        assert!((stated_percentile(&latencies, 0.95).unwrap() - 95.0).abs() < f64::EPSILON);
        assert!((stated_percentile(&latencies, 0.99).unwrap() - 99.0).abs() < f64::EPSILON);
    }

    #[test]
    fn exploration_spec_round_trips_yaml() {
        let spec = ExplorationSpec {
            schema_version: "mavai-explore-1".to_owned(),
            service_contract_id: "shopping-basket".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: Some("model-comparison".to_owned()),
            factors: BTreeMap::from([
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
                failure_distribution: Some(vec![FailureDistributionEntry {
                    condition: "relevance-check".to_owned(),
                    input_index: None,
                    input_excerpt: None,
                    count: 1,
                }]),
                criteria: None,
            },
            configuration: None,
            latency: None,
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        let restored = ExplorationSpec::from_yaml(&yaml).unwrap();

        assert_eq!(restored.schema_version, "mavai-explore-1");
        assert_eq!(restored.service_contract_id, "shopping-basket");
        assert_eq!(restored.statistics.successes, 4);
        assert_eq!(restored.factors.len(), 2);
    }

    #[test]
    fn yaml_uses_camel_case() {
        let spec = ExplorationSpec {
            schema_version: "mavai-explore-1".to_owned(),
            service_contract_id: "test".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: None,
            factors: BTreeMap::new(),
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
                criteria: None,
            },
            configuration: None,
            latency: None,
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(yaml.contains("schemaVersion"));
        assert!(yaml.contains("serviceContractId"));
        assert!(yaml.contains("samplesPlanned"));
    }

    #[test]
    fn empty_factors_omitted() {
        let spec = ExplorationSpec {
            schema_version: "mavai-explore-1".to_owned(),
            service_contract_id: "test".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: None,
            factors: BTreeMap::new(),
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
                criteria: None,
            },
            configuration: None,
            latency: None,
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(!yaml.contains("factors"));
    }

    #[test]
    fn factor_values_serialize_as_natural_types() {
        let spec = ExplorationSpec {
            schema_version: "mavai-explore-1".to_owned(),
            service_contract_id: "test".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: None,
            factors: BTreeMap::from([
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
                criteria: None,
            },
            configuration: None,
            latency: None,
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
            schema_version: "mavai-explore-1".to_owned(),
            service_contract_id: "test".to_owned(),
            generated_at: "2026-04-04T10:00:00Z".to_owned(),
            experiment_id: None,
            factors: BTreeMap::new(),
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
                criteria: None,
            },
            configuration: None,
            latency: None,
            cost: None,
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(!yaml.contains("standardError"));
        assert!(!yaml.contains("confidenceInterval"));
        assert!(!yaml.contains("minPassRate"));
    }
}
