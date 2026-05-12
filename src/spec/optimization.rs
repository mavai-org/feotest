//! Optimization YAML output: full iteration history plus convergence metadata.
//!
//! Optimize experiments produce a single YAML artefact per run that records
//! each iteration's factor, score, and sample statistics along with
//! convergence details (best iteration, termination reason). The schema is
//! normative and shared across javai frameworks — only the `schemaVersion`
//! identifier differs between implementations.
//!
//! Factor values are serialised through [`serde_yaml::Value`], so any factor
//! type that implements [`serde::Serialize`] round-trips naturally. Scalars
//! emit as their natural YAML type; multi-line strings emit as block scalars;
//! struct factors emit as YAML mappings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::experiment::{IterationRecord, Objective, OptimizeResult};

/// Canonical schema identifier for feotest optimization output.
pub const OPTIMIZATION_SCHEMA_VERSION: &str = "feotest-spec-1";

const DEFAULT_EXPERIMENT_ID: &str = "optimize";

/// One row of the `iterations` sequence in the optimization YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IterationBlock {
    /// Zero-indexed iteration number.
    pub iteration: u32,
    /// The factor used in this iteration, serialised as its natural
    /// YAML representation.
    pub factor_value: serde_yaml::Value,
    /// The score produced by the [`crate::experiment::Scorer`].
    pub score: f64,
    /// Successful trials in this iteration.
    pub successes: u32,
    /// Failed trials in this iteration.
    pub failures: u32,
    /// Total trials executed (successes + failures).
    pub samples_executed: u32,
}

impl IterationBlock {
    /// Builds a block from a typed [`IterationRecord`].
    ///
    /// # Panics
    ///
    /// Panics if the factor cannot be serialised via
    /// [`serde_yaml::to_value`]. This is a programmer error: every
    /// valid `Serialize` impl produces a valid `serde_yaml::Value`
    /// (strings, numbers, bools, sequences, and maps all round-trip).
    #[must_use]
    pub fn from_record<F: Serialize>(record: &IterationRecord<F>) -> Self {
        let factor_value = serde_yaml::to_value(record.factor())
            .expect("factor serialisation must not fail for a valid Serialize impl");
        Self {
            iteration: record.iteration(),
            factor_value,
            score: record.score(),
            successes: record.successes(),
            failures: record.failures(),
            samples_executed: record.successes() + record.failures(),
        }
    }
}

/// The `convergence` block in the optimization YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvergenceBlock {
    /// Number of iterations actually executed.
    pub total_iterations: u32,
    /// Iteration number of the best score (0-indexed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_iteration: Option<u32>,
    /// The best score achieved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best_score: Option<f64>,
    /// Why the run stopped iterating.
    pub termination_reason: String,
}

/// The complete optimization YAML document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationSpec {
    /// Schema version identifier.
    pub schema_version: String,
    /// The use case identifier.
    #[serde(rename = "useCaseId")]
    pub service_contract_id: String,
    /// The experiment identifier. Used as the YAML filename stem.
    pub experiment_id: String,
    /// "MAXIMIZE" or "MINIMIZE".
    pub objective: String,
    /// Full iteration history in execution order.
    pub iterations: Vec<IterationBlock>,
    /// Convergence summary.
    pub convergence: ConvergenceBlock,
}

impl OptimizationSpec {
    /// Builds a spec from an [`OptimizeResult`].
    ///
    /// When the result carries no experiment identifier, the stem
    /// defaults to `"optimize"` so the artefact always has a stable
    /// filename.
    #[must_use]
    pub fn from_result<F: Serialize>(result: &OptimizeResult<F>) -> Self {
        let iterations: Vec<IterationBlock> = result
            .history()
            .iter()
            .map(IterationBlock::from_record)
            .collect();
        let total_iterations = u32::try_from(iterations.len()).unwrap_or(u32::MAX);
        let objective = match result.objective() {
            Objective::Maximize => "MAXIMIZE",
            Objective::Minimize => "MINIMIZE",
        }
        .to_owned();

        Self {
            schema_version: OPTIMIZATION_SCHEMA_VERSION.to_owned(),
            service_contract_id: result.service_contract_id().to_owned(),
            experiment_id: result
                .experiment_id()
                .unwrap_or(DEFAULT_EXPERIMENT_ID)
                .to_owned(),
            objective,
            iterations,
            convergence: ConvergenceBlock {
                total_iterations,
                best_iteration: result.best_iteration(),
                best_score: result.best_score(),
                termination_reason: result.termination_reason().as_str().to_owned(),
            },
        }
    }

    /// Serialises the spec to YAML.
    ///
    /// # Errors
    ///
    /// Returns an error if YAML serialisation fails.
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Parses a spec from YAML.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is malformed or missing required
    /// fields.
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
}

/// Default root directory for feotest optimization output.
///
/// Places artefacts under `target/` so that running an optimisation
/// does not pollute the source tree. Projects that want to commit
/// optimisation histories as a historical record can pass a different
/// root to [`OptimizeSpecWriter::new`].
#[must_use]
pub fn default_output_root() -> PathBuf {
    PathBuf::from("target")
        .join("feotest")
        .join("optimizations")
}

/// Writes optimization YAML artefacts to disk.
///
/// Files land at `{root}/{service_contract_id}/{experiment_id}.yaml`, where
/// `root` defaults to [`default_output_root`].
pub struct OptimizeSpecWriter {
    root: PathBuf,
}

impl OptimizeSpecWriter {
    /// Creates a writer rooted at the given directory.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Creates a writer using the framework default output root.
    #[must_use]
    pub fn with_default_root() -> Self {
        Self::new(default_output_root())
    }

    /// The output root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Writes a spec built from the given result. Returns the written
    /// path.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation, YAML serialisation, or
    /// file writing fails.
    pub fn write<F: Serialize>(
        &self,
        result: &OptimizeResult<F>,
    ) -> Result<PathBuf, std::io::Error> {
        let spec = OptimizationSpec::from_result(result);
        self.write_spec(&spec)
    }

    /// Writes a pre-built spec. Returns the written path.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation, YAML serialisation, or
    /// file writing fails.
    pub fn write_spec(&self, spec: &OptimizationSpec) -> Result<PathBuf, std::io::Error> {
        let dir = self.root.join(&spec.service_contract_id);
        std::fs::create_dir_all(&dir)?;

        let path = dir.join(format!("{}.yaml", spec.experiment_id));
        let yaml = spec.to_yaml().map_err(std::io::Error::other)?;
        std::fs::write(&path, yaml)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> OptimizationSpec {
        OptimizationSpec {
            schema_version: OPTIMIZATION_SCHEMA_VERSION.to_owned(),
            service_contract_id: "shopping-basket".to_owned(),
            experiment_id: "prompt-tune-v1".to_owned(),
            objective: "MAXIMIZE".to_owned(),
            iterations: vec![
                IterationBlock {
                    iteration: 0,
                    factor_value: serde_yaml::Value::String(
                        "You are a helpful assistant.".to_owned(),
                    ),
                    score: 0.65,
                    successes: 13,
                    failures: 7,
                    samples_executed: 20,
                },
                IterationBlock {
                    iteration: 1,
                    factor_value: serde_yaml::Value::String(
                        "You are a shopping assistant.".to_owned(),
                    ),
                    score: 0.8,
                    successes: 16,
                    failures: 4,
                    samples_executed: 20,
                },
            ],
            convergence: ConvergenceBlock {
                total_iterations: 2,
                best_iteration: Some(1),
                best_score: Some(0.8),
                termination_reason: "NO_IMPROVEMENT".to_owned(),
            },
        }
    }

    #[test]
    fn yaml_uses_camel_case_field_names() {
        let yaml = sample_spec().to_yaml().unwrap();
        assert!(yaml.contains("schemaVersion"));
        assert!(yaml.contains("useCaseId"));
        assert!(yaml.contains("experimentId"));
        assert!(yaml.contains("factorValue"));
        assert!(yaml.contains("samplesExecuted"));
        assert!(yaml.contains("totalIterations"));
        assert!(yaml.contains("bestIteration"));
        assert!(yaml.contains("bestScore"));
        assert!(yaml.contains("terminationReason"));
    }

    #[test]
    fn yaml_round_trips_through_deserialise() {
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();
        let restored = OptimizationSpec::from_yaml(&yaml).unwrap();

        assert_eq!(restored.schema_version, spec.schema_version);
        assert_eq!(restored.iterations.len(), 2);
        assert_eq!(restored.convergence.total_iterations, 2);
        assert_eq!(restored.convergence.best_iteration, Some(1));
        assert_eq!(restored.convergence.termination_reason, "NO_IMPROVEMENT");
    }

    #[test]
    fn multi_line_string_factor_uses_block_scalar() {
        let spec = OptimizationSpec {
            iterations: vec![IterationBlock {
                iteration: 0,
                factor_value: serde_yaml::Value::String(
                    "You are a helpful assistant.\nAlways be polite.\nReturn JSON.".to_owned(),
                ),
                score: 1.0,
                successes: 5,
                failures: 0,
                samples_executed: 5,
            }],
            ..sample_spec()
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(
            yaml.contains("factorValue: |") || yaml.contains("factorValue: |-"),
            "expected block scalar for multi-line factor value; got:\n{yaml}"
        );
        assert!(
            !yaml.contains("\\n"),
            "multi-line factor should not contain \\n escapes:\n{yaml}"
        );
    }

    #[test]
    fn scalar_factor_values_serialise_as_natural_yaml_types() {
        let spec = OptimizationSpec {
            iterations: vec![
                IterationBlock {
                    iteration: 0,
                    factor_value: serde_yaml::to_value(0.7f64).unwrap(),
                    score: 0.9,
                    successes: 9,
                    failures: 1,
                    samples_executed: 10,
                },
                IterationBlock {
                    iteration: 1,
                    factor_value: serde_yaml::to_value(4i64).unwrap(),
                    score: 0.8,
                    successes: 8,
                    failures: 2,
                    samples_executed: 10,
                },
                IterationBlock {
                    iteration: 2,
                    factor_value: serde_yaml::to_value(true).unwrap(),
                    score: 0.95,
                    successes: 19,
                    failures: 1,
                    samples_executed: 20,
                },
            ],
            ..sample_spec()
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(yaml.contains("factorValue: 0.7"));
        assert!(yaml.contains("factorValue: 4"));
        assert!(yaml.contains("factorValue: true"));
    }

    #[test]
    fn struct_factor_serialises_as_yaml_mapping() {
        #[derive(Serialize)]
        struct ModelAndTemp {
            model: &'static str,
            temperature: f64,
        }

        let spec = OptimizationSpec {
            iterations: vec![IterationBlock {
                iteration: 0,
                factor_value: serde_yaml::to_value(ModelAndTemp {
                    model: "gpt-4",
                    temperature: 0.7,
                })
                .unwrap(),
                score: 0.9,
                successes: 9,
                failures: 1,
                samples_executed: 10,
            }],
            ..sample_spec()
        };

        let yaml = spec.to_yaml().unwrap();
        assert!(
            yaml.contains("model: gpt-4"),
            "expected struct factor to emit as mapping; got:\n{yaml}"
        );
        assert!(yaml.contains("temperature: 0.7"));
    }

    #[test]
    fn default_output_root_is_under_target() {
        let root = default_output_root();
        assert!(root.starts_with("target"));
    }
}
