//! Measure experiment: establishing precise empirical baselines.

use crate::controls::{ExecutionConfig, TokenRecorder};
use crate::experiment::engine::{ExecutionEngine, ExecutionResult};
use crate::model::TrialOutcome;
use crate::spec::SpecResolver;
use crate::spec::baseline::{
    BaselineSpec, CostBlock, ExecutionBlock, RequirementsBlock, StatisticsBlock, SuccessRateBlock,
};
use crate::statistics::types::ConfidenceLevel;
use crate::statistics::{defaults, proportion};

/// A measure experiment that runs many samples to establish a precise baseline.
///
/// The resulting baseline spec contains the observed success rate, confidence
/// interval, and a derived minimum pass rate (Wilson lower bound at 95%
/// confidence).
///
/// # Examples
///
/// ```no_run
/// use feotest::experiment::MeasureExperiment;
/// use feotest::model::TrialOutcome;
/// use std::time::Duration;
///
/// let result = MeasureExperiment::new(
///     "my-service",
///     1000,
///     &["input-1".to_string(), "input-2".to_string()],
///     |input| TrialOutcome::success(Duration::from_millis(10)),
/// )
/// .run();
/// ```
pub struct MeasureExperiment<'a, F> {
    use_case_id: String,
    config: ExecutionConfig,
    inputs: &'a [String],
    trial: F,
    experiment_id: Option<String>,
    spec_resolver: Option<SpecResolver>,
}

impl<'a, F> MeasureExperiment<'a, F>
where
    F: FnMut(&str) -> TrialOutcome,
{
    /// Creates a new measure experiment.
    pub fn new(
        use_case_id: impl Into<String>,
        samples: u32,
        inputs: &'a [String],
        trial: F,
    ) -> Self {
        Self {
            use_case_id: use_case_id.into(),
            config: ExecutionConfig::new(samples),
            inputs,
            trial,
            experiment_id: None,
            spec_resolver: None,
        }
    }

    /// Sets the execution configuration (overrides sample count from constructor).
    #[must_use]
    pub const fn with_config(mut self, config: ExecutionConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the experiment identifier.
    #[must_use]
    pub fn with_experiment_id(mut self, id: impl Into<String>) -> Self {
        self.experiment_id = Some(id.into());
        self
    }

    /// Sets a spec resolver for writing the baseline spec to disk.
    #[must_use]
    pub fn with_spec_resolver(mut self, resolver: SpecResolver) -> Self {
        self.spec_resolver = Some(resolver);
        self
    }

    /// Runs the measure experiment and returns the result.
    pub fn run(mut self) -> MeasureResult {
        let token_recorder = TokenRecorder::new();
        let result =
            ExecutionEngine::run(&self.config, self.inputs, &token_recorder, &mut self.trial);

        let spec = self.build_spec(&result);

        // Write spec to disk if resolver is configured
        let spec_path = self
            .spec_resolver
            .as_ref()
            .and_then(|resolver| resolver.write(&spec).ok());

        MeasureResult {
            execution: result,
            spec,
            spec_path,
        }
    }

    fn build_spec(&self, result: &ExecutionResult) -> BaselineSpec {
        let summary = result.summary();
        let successes = summary.successes();
        let total = summary.samples_executed();
        let failures = summary.failures();

        let observed_rate = summary.observed_pass_rate();
        let confidence = ConfidenceLevel::new(defaults::DEFAULT_CONFIDENCE);

        // Compute Wilson score interval
        let estimate = proportion::estimate(successes, total, confidence);

        // Wilson lower bound as the derived threshold
        let lower_bound = proportion::lower_bound(successes, total, confidence);

        let se = proportion::standard_error(successes, total);

        // Build failure distribution
        let failure_dist = if result.aggregate().failure_distribution().is_empty() {
            None
        } else {
            let mut map = std::collections::BTreeMap::new();
            for (check, count) in result.aggregate().failure_distribution() {
                map.insert(check.clone(), *count);
            }
            Some(map)
        };

        let now = chrono_like_now();

        let mut spec = BaselineSpec::new(
            &self.use_case_id,
            &now,
            ExecutionBlock {
                samples_planned: self.config.samples(),
                samples_executed: total,
                termination_reason: Some(summary.termination().reason().to_string()),
            },
            RequirementsBlock {
                min_pass_rate: round4(lower_bound),
            },
            StatisticsBlock {
                success_rate: SuccessRateBlock {
                    observed: round4(observed_rate),
                    standard_error: round4(se),
                    confidence_interval95: [
                        round4(estimate.lower_bound()),
                        round4(estimate.upper_bound()),
                    ],
                },
                successes,
                failures,
                failure_distribution: failure_dist,
            },
        );

        spec.experiment_id.clone_from(&self.experiment_id);

        let cost = summary.cost();
        spec.cost = Some(CostBlock {
            total_time_ms: u64::try_from(cost.total_time().as_millis()).unwrap_or(u64::MAX),
            avg_time_per_sample_ms: u64::try_from(cost.avg_time_per_sample().as_millis())
                .unwrap_or(u64::MAX),
            total_tokens: cost.total_tokens(),
            avg_tokens_per_sample: cost.avg_tokens_per_sample(),
        });

        spec
    }
}

/// Round to 4 decimal places for spec output.
fn round4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

/// Simple ISO 8601 timestamp (no chrono dependency).
fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Basic formatting without chrono
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Approximate date from days since epoch
    // Good enough for spec timestamps
    let (year, month, day) = days_to_date(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Converts days since Unix epoch to (year, month, day).
const fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Civil date algorithm from Howard Hinnant
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Result of a measure experiment.
#[derive(Debug)]
pub struct MeasureResult {
    execution: ExecutionResult,
    spec: BaselineSpec,
    spec_path: Option<std::path::PathBuf>,
}

impl MeasureResult {
    /// The execution result.
    #[must_use]
    pub const fn execution(&self) -> &ExecutionResult {
        &self.execution
    }

    /// The generated baseline spec.
    #[must_use]
    pub const fn spec(&self) -> &BaselineSpec {
        &self.spec
    }

    /// Path where the spec was written, if a resolver was configured.
    #[must_use]
    pub fn spec_path(&self) -> Option<&std::path::Path> {
        self.spec_path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn succeeding_trial(_input: &str) -> TrialOutcome {
        TrialOutcome::success(Duration::from_millis(1))
    }

    #[test]
    fn produces_baseline_spec() {
        let inputs = vec!["input".to_string()];
        let result = MeasureExperiment::new("test-service", 100, &inputs, succeeding_trial)
            .with_experiment_id("baseline-v1")
            .run();

        let spec = result.spec();
        assert_eq!(spec.use_case_id, "test-service");
        assert_eq!(spec.experiment_id.as_deref(), Some("baseline-v1"));
        assert_eq!(spec.statistics.successes, 100);
        assert_eq!(spec.statistics.failures, 0);
        assert!(spec.requirements.min_pass_rate > 0.9);
        assert!(spec.cost.is_some());
    }

    #[test]
    fn writes_spec_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());
        let inputs = vec!["input".to_string()];

        let result = MeasureExperiment::new("disk-test", 50, &inputs, succeeding_trial)
            .with_spec_resolver(resolver)
            .run();

        assert!(result.spec_path().is_some());
        assert!(result.spec_path().unwrap().exists());
    }

    #[test]
    fn tracks_failure_distribution() {
        let inputs = vec!["input".to_string()];
        let mut call_count = 0u32;
        let result = MeasureExperiment::new("mixed-service", 10, &inputs, |_input| {
            call_count += 1;
            if call_count % 3 == 0 {
                TrialOutcome::failure(
                    crate::model::ContractViolation::new("parse", "bad json"),
                    Duration::from_millis(1),
                )
            } else {
                TrialOutcome::success(Duration::from_millis(1))
            }
        })
        .run();

        let spec = result.spec();
        assert!(spec.statistics.failures > 0);
        assert!(spec.statistics.failure_distribution.is_some());
    }

    #[test]
    fn round4_works() {
        assert!((round4(0.123_456_789) - 0.1235).abs() < 1e-10);
        assert!((round4(0.5) - 0.5).abs() < 1e-10);
    }
}
