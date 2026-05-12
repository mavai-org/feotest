//! A built, ready-to-run probabilistic test.
//!
//! [`ProbabilisticTest`] is the result of
//! [`ProbabilisticTestBuilder::build`](crate::ptest::ProbabilisticTestBuilder::build).
//! It holds the fully validated configuration and exposes a single
//! [`run`](ProbabilisticTest::run) method that executes the test and
//! returns a [`ProbabilisticTestResult`].

use std::path::PathBuf;
use std::time::Duration;

use crate::controls::{ExecutionConfig, PacingConfig};
use crate::latency::{LatencyEnforcementMode, LatencyThresholds};
use crate::model::{BudgetExhaustedBehavior, TestIntent, ThresholdOrigin, TrialOutcome};
use crate::ptest::builder::{
    ThresholdApproach, build_default_spec_resolver, macro_config_from_approach,
};
use crate::ptest::runner::{
    self, AssessmentCriteria, BaselineContext, LatencyConfig, ProbabilisticTestResult,
};
use crate::ptest::validation;
use crate::spec::{BaselineSpec, SpecResolver};
use crate::service_contract::CovariateContext;

/// A built probabilistic test.
///
/// Construct via [`crate::ptest::ProbabilisticTestBuilder::builder`]
/// followed by `.build()`. Execute via [`run`](Self::run).
pub struct ProbabilisticTest<'a, T> {
    pub(crate) service_contract_id: String,
    pub(crate) factory: Box<dyn Fn() -> T + 'a>,
    pub(crate) inputs: &'a [String],
    pub(crate) trial: Box<dyn FnMut(&T, &str) -> TrialOutcome + 'a>,

    pub(crate) approach: ThresholdApproach,
    pub(crate) intent: TestIntent,
    pub(crate) threshold_origin: ThresholdOrigin,
    pub(crate) contract_ref: Option<String>,

    pub(crate) spec_resolver: Option<SpecResolver>,
    pub(crate) baseline_spec: Option<BaselineSpec>,
    pub(crate) baseline_path: Option<PathBuf>,
    pub(crate) baseline_dir: Option<PathBuf>,

    pub(crate) config_overrides: Option<ExecutionConfig>,
    pub(crate) time_budget: Option<Duration>,
    pub(crate) token_budget: Option<u64>,
    pub(crate) pacing: Option<PacingConfig>,
    pub(crate) on_budget_exhausted: Option<BudgetExhaustedBehavior>,

    pub(crate) transparent_stats: bool,
    pub(crate) covariate_context: Option<CovariateContext>,

    pub(crate) latency_thresholds: LatencyThresholds,
    pub(crate) baseline_latency_mode: Option<LatencyEnforcementMode>,
    pub(crate) baseline_latency_confidence: Option<f64>,
    pub(crate) fail_on_expired_baseline: bool,
}

impl<T> ProbabilisticTest<'_, T> {
    /// Runs the probabilistic test and returns the result.
    ///
    /// The factory is invoked once to produce the service contract instance,
    /// the trials run against it, the verdict is assembled, and the
    /// console verdict line is printed. The result is returned
    /// regardless of outcome — callers decide what to do with a `Fail`.
    pub fn run(self) -> ProbabilisticTestResult {
        let spec_resolver = resolve_spec_resolver(&self);
        let has_baseline = self.baseline_spec.is_some() || spec_resolver.is_some();

        let config = macro_config_from_approach(
            &self.service_contract_id,
            &self.approach,
            self.threshold_origin,
            has_baseline,
        );
        validation::validate(&config);

        let transparent_stats = self.transparent_stats;
        let criteria = AssessmentCriteria {
            approach: self.approach,
            intent: self.intent,
            threshold_origin: self.threshold_origin,
            contract_ref: self.contract_ref,
            latency: LatencyConfig {
                thresholds: self.latency_thresholds,
                baseline_mode: self.baseline_latency_mode,
                baseline_confidence: self
                    .baseline_latency_confidence
                    .unwrap_or(crate::latency::DEFAULT_BASELINE_CONFIDENCE),
            },
            fail_on_expired_baseline: self.fail_on_expired_baseline,
            on_budget_exhausted: self.on_budget_exhausted,
        };
        let baseline = BaselineContext {
            spec_resolver,
            pre_resolved_spec: self.baseline_spec,
            covariate_context: self.covariate_context,
        };

        let config_overrides = self.config_overrides.or_else(|| {
            build_config_overrides(
                &criteria.approach,
                self.time_budget,
                self.token_budget,
                self.pacing.as_ref(),
                self.on_budget_exhausted,
            )
        });

        let service_contract = (self.factory)();
        let mut trial = self.trial;
        let trial_adapter = move |input: &str| trial(&service_contract, input);

        let result = runner::execute(
            &self.service_contract_id,
            self.inputs,
            trial_adapter,
            &criteria,
            baseline,
            config_overrides.as_ref(),
        );

        let renderer = crate::reporting::ConsoleRenderer::new();
        renderer.print_verdict(result.verdict_record());

        if transparent_stats {
            let mut buf = String::new();
            crate::reporting::transparent::render(
                result.verdict_record(),
                result.approach(),
                &mut buf,
            )
            .expect("formatting should not fail");
            eprint!("{buf}");
        }

        result
    }
}

/// Determines the effective spec resolver for a test.
///
/// A resolver is needed when:
/// - the approach requires a baseline (no explicit threshold), or
/// - covariates are declared (baseline must be loaded for integrity), or
/// - the user explicitly supplied a baseline path or directory.
fn resolve_spec_resolver<T>(test: &ProbabilisticTest<'_, T>) -> Option<SpecResolver> {
    if let Some(resolver) = &test.spec_resolver {
        return Some(resolver.clone());
    }
    if test.baseline_spec.is_some() {
        return None;
    }

    let needs_baseline = !matches!(test.approach, ThresholdApproach::ThresholdFirst { .. },);
    let has_covariates = test.covariate_context.is_some();
    let user_specified_location = test.baseline_path.is_some() || test.baseline_dir.is_some();

    if !needs_baseline && !has_covariates && !user_specified_location {
        return None;
    }

    Some(build_default_spec_resolver(
        test.baseline_path.as_deref(),
        test.baseline_dir.as_deref(),
    ))
}

/// Builds optional execution config overrides from the simplified
/// budget/pacing setters.
fn build_config_overrides(
    approach: &ThresholdApproach,
    time_budget: Option<Duration>,
    token_budget: Option<u64>,
    pacing: Option<&PacingConfig>,
    on_budget_exhausted: Option<BudgetExhaustedBehavior>,
) -> Option<ExecutionConfig> {
    if time_budget.is_none()
        && token_budget.is_none()
        && pacing.is_none()
        && on_budget_exhausted.is_none()
    {
        return None;
    }

    let samples = match approach {
        ThresholdApproach::ThresholdFirst { samples, .. }
        | ThresholdApproach::SampleSizeFirst { samples, .. } => *samples,
        // Confidence-first computes samples at runtime. The runner
        // synthesises its own config in that case; we cannot pre-compute
        // here.
        ThresholdApproach::ConfidenceFirst { .. } => return None,
    };

    let mut config = ExecutionConfig::new(samples);
    if let Some(budget) = time_budget {
        config = config.with_time_budget(budget);
    }
    if let Some(budget) = token_budget {
        config = config.with_token_budget(budget);
    }
    if let Some(p) = pacing {
        config = config.pacing(p.clone());
    }
    if let Some(behaviour) = on_budget_exhausted {
        config = config.with_on_budget_exhausted(behaviour);
    }
    Some(config)
}
