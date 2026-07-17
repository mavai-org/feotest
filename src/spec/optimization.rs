//! Optimization YAML output: the family's canonical optimize artefact.
//!
//! Each optimize run produces a single YAML document in the canonical
//! `mavai-optimize-1` interchange format: the objective, the full iteration
//! history — every configuration tried, its score, and its descriptive
//! statistics — and the convergence summary naming the selected optimum.
//! Optimization output is descriptive, not inferential: scores and rates
//! are observed values; no claim is made about the optimum beyond the
//! scorer's ordering.
//!
//! Latency percentiles are **stated value-or-absent**, exactly as on the
//! exploration side: each is emitted only when the iteration's passing
//! samples clear that percentile's minimum-sample floor. At optimization's
//! typically small per-iteration counts most are absent — that is the gate
//! working. Consumers render absence; they never compute a replacement.
//!
//! Factor values are serialised through [`serde_yaml::Value`]: a struct
//! factor emits as the `factors` mapping directly; a scalar factor emits
//! under the key `factor`. Multi-line strings emit as block scalars.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::experiment::{IterationObservation, IterationRecord, Objective, OptimizeResult};
use crate::spec::baseline::{CostBlock, ExecutionBlock};
use crate::spec::common::{build_cost_block, build_failure_distribution, now_iso8601, round4};
use crate::spec::explore::{
    ExplorationLatencyBlock, ExplorationStatisticsBlock, build_criteria_blocks, build_latency_block,
};

/// Canonical schema identifier for optimization output.
pub const OPTIMIZATION_SCHEMA_VERSION: &str = "mavai-optimize-1";

const DEFAULT_EXPERIMENT_ID: &str = "optimize";

/// One row of the `iterations` sequence in the optimization YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IterationBlock {
    /// Zero-indexed iteration number.
    pub iteration: u32,
    /// The iteration's factor values, as a YAML mapping.
    pub factors: serde_yaml::Value,
    /// The scoring function's value for this iteration.
    pub score: f64,
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

impl IterationBlock {
    fn build<F: Serialize>(
        record: &IterationRecord<F>,
        observation: &IterationObservation,
    ) -> Self {
        let execution = observation.execution();
        let summary = execution.summary();
        Self {
            iteration: record.iteration(),
            factors: factors_mapping(record.factor()),
            score: record.score(),
            execution: ExecutionBlock {
                samples_planned: summary.samples_planned(),
                samples_executed: summary.samples_executed(),
                termination_reason: Some(summary.termination().reason().to_string()),
            },
            statistics: ExplorationStatisticsBlock {
                observed: round4(summary.observed_pass_rate()),
                successes: summary.successes(),
                failures: summary.failures(),
                failure_distribution: build_failure_distribution(execution.aggregate()),
                criteria: build_criteria_blocks(execution),
            },
            latency: build_latency_block(observation.projections(), summary.samples_executed()),
            cost: Some(build_cost_block(summary.cost())),
        }
    }
}

/// The convergence summary naming the selected optimum.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvergenceBlock {
    /// Iterations executed.
    pub total_iterations: u32,
    /// Index of the selected optimum in `iterations`.
    pub best_iteration: u32,
    /// The selected optimum's score.
    pub best_score: f64,
    /// The selected optimum's factor values — equal to the named
    /// iteration's `factors`.
    pub best_factors: serde_yaml::Value,
    /// Why the run stopped iterating.
    pub termination_reason: String,
}

/// The complete optimization YAML document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// javai-ref: JVI-FJK9SN9 — do not remove (resolves in mavai-orchestrator)
pub struct OptimizationSpec {
    /// Schema version identifier: `mavai-optimize-1`.
    pub schema_version: String,
    /// The service contract identifier.
    pub service_contract_id: String,
    /// The experiment identifier. Used as the YAML filename stem.
    pub experiment_id: String,
    /// "MAXIMIZE" or "MINIMIZE".
    pub objective: String,
    /// The scoring function's stable domain name, when the run's scorer
    /// carries one. Absent for a bespoke unnamed scorer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scorer: Option<String>,
    /// ISO 8601 timestamp of when the document was generated.
    pub generated_at: String,
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
    ///
    /// # Panics
    ///
    /// Panics if the result's history is empty — an optimize run always
    /// executes iteration 0, so an empty history is a defect in the
    /// calling code, not an emittable state.
    #[must_use]
    pub fn from_result<F: Serialize>(result: &OptimizeResult<F>) -> Self {
        assert!(
            !result.history().is_empty(),
            "an optimize run always executes iteration 0; an empty history is not emittable"
        );
        let iterations: Vec<IterationBlock> = result
            .history()
            .iter()
            .zip(result.observations())
            .map(|(record, observation)| IterationBlock::build(record, observation))
            .collect();
        let total_iterations = u32::try_from(iterations.len()).unwrap_or(u32::MAX);
        let objective = match result.objective() {
            Objective::Maximize => "MAXIMIZE",
            Objective::Minimize => "MINIMIZE",
        }
        .to_owned();
        let best_iteration = result
            .best_iteration()
            .expect("a non-empty history always names a best iteration");
        let best = &iterations[best_iteration as usize];

        Self {
            schema_version: OPTIMIZATION_SCHEMA_VERSION.to_owned(),
            service_contract_id: result.service_contract_id().to_owned(),
            experiment_id: result
                .experiment_id()
                .unwrap_or(DEFAULT_EXPERIMENT_ID)
                .to_owned(),
            objective,
            scorer: result.scorer_name().map(str::to_owned),
            generated_at: now_iso8601(),
            convergence: ConvergenceBlock {
                total_iterations,
                best_iteration,
                best_score: best.score,
                best_factors: best.factors.clone(),
                termination_reason: result.termination_reason().as_str().to_owned(),
            },
            iterations,
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

/// The `factors` mapping for a factor value: a struct factor's own
/// mapping, or a scalar factor keyed as `factor`.
fn factors_mapping<F: Serialize>(factor: &F) -> serde_yaml::Value {
    let value = serde_yaml::to_value(factor)
        .expect("factor types are plain data and must serialise to YAML");
    match value {
        mapping @ serde_yaml::Value::Mapping(_) => mapping,
        scalar => {
            let mut mapping = serde_yaml::Mapping::new();
            mapping.insert(serde_yaml::Value::String("factor".to_owned()), scalar);
            serde_yaml::Value::Mapping(mapping)
        }
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
// javai-ref: JVI-FJK9SN9 — do not remove (resolves in mavai-orchestrator)
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
    ///
    /// # Panics
    ///
    /// Panics if the result's history is empty (see
    /// [`OptimizationSpec::from_result`]).
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

    fn iteration_block(iteration: u32, prompt: &str, score: f64) -> IterationBlock {
        let mut factors = serde_yaml::Mapping::new();
        factors.insert(
            serde_yaml::Value::String("system-prompt".to_owned()),
            serde_yaml::Value::String(prompt.to_owned()),
        );
        IterationBlock {
            iteration,
            factors: serde_yaml::Value::Mapping(factors),
            score,
            execution: ExecutionBlock {
                samples_planned: 20,
                samples_executed: 20,
                termination_reason: Some("COMPLETED".to_owned()),
            },
            statistics: ExplorationStatisticsBlock {
                observed: score,
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "test fixture: small non-negative counts"
                )]
                successes: (score * 20.0) as u32,
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "test fixture: small non-negative counts"
                )]
                failures: 20 - (score * 20.0) as u32,
                failure_distribution: None,
                criteria: None,
            },
            latency: None,
            cost: None,
        }
    }

    fn sample_spec() -> OptimizationSpec {
        let iterations = vec![
            iteration_block(0, "You are a helpful assistant.", 0.65),
            iteration_block(1, "You are a shopping assistant.", 0.8),
        ];
        let best_factors = iterations[1].factors.clone();
        OptimizationSpec {
            schema_version: OPTIMIZATION_SCHEMA_VERSION.to_owned(),
            service_contract_id: "shopping-basket".to_owned(),
            experiment_id: "prompt-tune-v1".to_owned(),
            objective: "MAXIMIZE".to_owned(),
            scorer: Some("observed-pass-rate".to_owned()),
            generated_at: "2026-07-17T10:00:00Z".to_owned(),
            iterations,
            convergence: ConvergenceBlock {
                total_iterations: 2,
                best_iteration: 1,
                best_score: 0.8,
                best_factors,
                termination_reason: "NO_IMPROVEMENT".to_owned(),
            },
        }
    }

    #[test]
    fn yaml_uses_the_canonical_camel_case_field_names() {
        let yaml = sample_spec().to_yaml().unwrap();
        assert!(yaml.contains("schemaVersion: mavai-optimize-1"));
        assert!(yaml.contains("serviceContractId"));
        assert!(!yaml.contains("useCaseId"));
        assert!(yaml.contains("experimentId"));
        assert!(yaml.contains("scorer: observed-pass-rate"));
        assert!(yaml.contains("generatedAt"));
        assert!(yaml.contains("factors"));
        assert!(yaml.contains("samplesExecuted"));
        assert!(yaml.contains("totalIterations"));
        assert!(yaml.contains("bestIteration"));
        assert!(yaml.contains("bestScore"));
        assert!(yaml.contains("bestFactors"));
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
        assert_eq!(restored.convergence.best_iteration, 1);
        assert_eq!(
            restored.convergence.best_factors,
            spec.convergence.best_factors
        );
    }

    #[test]
    fn an_unnamed_scorer_leaves_the_field_absent() {
        let mut spec = sample_spec();
        spec.scorer = None;
        let yaml = spec.to_yaml().unwrap();
        assert!(!yaml.contains("scorer"));
    }

    #[test]
    fn a_scalar_factor_maps_under_the_factor_key() {
        let value = factors_mapping(&0.7_f64);
        let serde_yaml::Value::Mapping(mapping) = value else {
            panic!("expected a mapping");
        };
        assert_eq!(
            mapping.get(serde_yaml::Value::String("factor".to_owned())),
            Some(&serde_yaml::Value::Number(0.7.into()))
        );
    }
}
