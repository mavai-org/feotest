//! Optimization YAML output: full iteration history plus convergence metadata.
//!
//! Optimize experiments produce a single YAML artefact per run that records
//! each iteration's factor value, score, and sample statistics along with
//! convergence details (best iteration, termination reason). The schema is
//! normative and shared across javai frameworks — only the `schemaVersion`
//! identifier differs between implementations.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::experiment::{IterationRecord, Objective, OptimizeResult};
use crate::usecase::FactorValue;

/// Canonical schema identifier for feotest optimization output.
pub const OPTIMIZATION_SCHEMA_VERSION: &str = "feotest-spec-1";

const DEFAULT_EXPERIMENT_ID: &str = "optimize";

/// A serde-friendly wrapper for factor values in optimization YAML output.
///
/// Serializes as the natural YAML type without enum tags. Multi-line strings
/// are emitted as YAML block scalars by the underlying serializer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OptimizationFactorValue {
    /// A string factor value.
    String(String),
    /// A floating-point factor value.
    Float(f64),
    /// An integer factor value.
    Int(i64),
    /// A boolean factor value.
    Bool(bool),
}

impl From<&FactorValue> for OptimizationFactorValue {
    fn from(value: &FactorValue) -> Self {
        match value {
            FactorValue::String(s) => Self::String(s.clone()),
            FactorValue::Float(v) => Self::Float(*v),
            FactorValue::Int(v) => Self::Int(*v),
            FactorValue::Bool(v) => Self::Bool(*v),
        }
    }
}

/// One row of the `iterations` sequence in the optimization YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IterationBlock {
    /// Zero-indexed iteration number.
    pub iteration: u32,
    /// The control factor value used in this iteration.
    pub factor_value: OptimizationFactorValue,
    /// The score produced by the [`crate::experiment::Scorer`].
    pub score: f64,
    /// Successful trials in this iteration.
    pub successes: u32,
    /// Failed trials in this iteration.
    pub failures: u32,
    /// Total trials executed (successes + failures).
    pub samples_executed: u32,
}

impl From<&IterationRecord> for IterationBlock {
    fn from(record: &IterationRecord) -> Self {
        Self {
            iteration: record.iteration(),
            factor_value: OptimizationFactorValue::from(record.factor_value()),
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
    pub use_case_id: String,
    /// The experiment identifier. Used as the YAML filename stem.
    pub experiment_id: String,
    /// Name of the factor that was optimised.
    pub control_factor: String,
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
    /// When the result carries no experiment identifier, the stem defaults to
    /// `"optimize"` so the artefact always has a stable filename.
    #[must_use]
    pub fn from_result(result: &OptimizeResult) -> Self {
        let iterations: Vec<IterationBlock> =
            result.history().iter().map(IterationBlock::from).collect();
        let total_iterations = u32::try_from(iterations.len()).unwrap_or(u32::MAX);
        let objective = match result.objective() {
            Objective::Maximize => "MAXIMIZE",
            Objective::Minimize => "MINIMIZE",
        }
        .to_owned();

        Self {
            schema_version: OPTIMIZATION_SCHEMA_VERSION.to_owned(),
            use_case_id: result.use_case_id().to_owned(),
            experiment_id: result
                .experiment_id()
                .unwrap_or(DEFAULT_EXPERIMENT_ID)
                .to_owned(),
            control_factor: result.control_factor().to_owned(),
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
    /// Returns an error if the input is malformed or missing required fields.
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
}

/// Default root directory for feotest optimization output.
///
/// Places artefacts under `target/` so that running an optimisation does not
/// pollute the source tree. Projects that want to commit optimisation
/// histories as a historical record can pass a different root to
/// [`OptimizeSpecWriter::new`].
#[must_use]
pub fn default_output_root() -> PathBuf {
    PathBuf::from("target")
        .join("feotest")
        .join("optimizations")
}

/// Writes optimization YAML artefacts to disk.
///
/// Files land at `{root}/{use_case_id}/{experiment_id}.yaml`, where `root`
/// defaults to [`default_output_root`].
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

    /// Writes a spec built from the given result. Returns the written path.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation, YAML serialisation, or
    /// file writing fails.
    pub fn write(&self, result: &OptimizeResult) -> Result<PathBuf, std::io::Error> {
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
        let dir = self.root.join(&spec.use_case_id);
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
            use_case_id: "shopping-basket".to_owned(),
            experiment_id: "prompt-tune-v1".to_owned(),
            control_factor: "systemPrompt".to_owned(),
            objective: "MAXIMIZE".to_owned(),
            iterations: vec![
                IterationBlock {
                    iteration: 0,
                    factor_value: OptimizationFactorValue::String(
                        "You are a helpful assistant.".to_owned(),
                    ),
                    score: 0.65,
                    successes: 13,
                    failures: 7,
                    samples_executed: 20,
                },
                IterationBlock {
                    iteration: 1,
                    factor_value: OptimizationFactorValue::String(
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
        assert!(yaml.contains("controlFactor"));
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
                factor_value: OptimizationFactorValue::String(
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
        // Block scalar marker for literal style.
        assert!(
            yaml.contains("factorValue: |") || yaml.contains("factorValue: |-"),
            "expected block scalar for multi-line factor value; got:\n{yaml}"
        );
        // Body must not be serialised as quoted escape sequences.
        assert!(
            !yaml.contains("\\n"),
            "multi-line factor should not contain \\n escapes:\n{yaml}"
        );
    }

    #[test]
    fn factor_values_serialise_as_natural_yaml_types() {
        let spec = OptimizationSpec {
            iterations: vec![
                IterationBlock {
                    iteration: 0,
                    factor_value: OptimizationFactorValue::Float(0.7),
                    score: 0.9,
                    successes: 9,
                    failures: 1,
                    samples_executed: 10,
                },
                IterationBlock {
                    iteration: 1,
                    factor_value: OptimizationFactorValue::Int(4),
                    score: 0.8,
                    successes: 8,
                    failures: 2,
                    samples_executed: 10,
                },
                IterationBlock {
                    iteration: 2,
                    factor_value: OptimizationFactorValue::Bool(true),
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
    fn default_output_root_is_under_target() {
        let root = default_output_root();
        assert!(root.starts_with("target"));
    }
}
