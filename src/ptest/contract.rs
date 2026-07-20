//! Contract-driven probabilistic test entry point.
//!
//! [`ProbabilisticTest::for_contract`] binds a test to a [`ServiceContract`]:
//! the engine invokes the contract and judges every criterion on every sample,
//! and the verdict decomposes per criterion with a composite over them.
//!
//! ```no_run
//! use feotest::ptest::ProbabilisticTest;
//! use feotest::ptest::builder::ThresholdApproach;
//! # fn run<C>(contract: C, inputs: &[String])
//! # where C: feotest::service_contract::ServiceContract<Input = String, Output = String> {
//! let result = ProbabilisticTest::for_contract(contract)
//!     .inputs(inputs)
//!     .approach(ThresholdApproach::ThresholdFirst { samples: 200, min_pass_rate: 0.90 })
//!     .run();
//! # let _ = result.verdict_record();
//! # }
//! ```

use std::path::PathBuf;
use std::time::Duration;

use crate::controls::{ExecutionConfig, PacingConfig};
use crate::latency::LatencyEnforcementMode;
use crate::model::{BudgetExhaustedBehavior, TestIntent, ThresholdOrigin};
use crate::ptest::ProbabilisticTest;
use crate::ptest::builder::{ThresholdApproach, build_default_spec_resolver};
use crate::ptest::probabilistic_test::build_config_overrides;
use crate::ptest::runner::{
    self, AssessmentCriteria, BaselineContext, LatencyConfig, ProbabilisticTestResult,
};
use crate::service_contract::{CovariateContext, ServiceContract};
use crate::spec::{BaselineSpec, SpecResolver};

/// Default sampling plan when the caller does not set an approach.
const DEFAULT_APPROACH: ThresholdApproach = ThresholdApproach::ThresholdFirst {
    samples: 100,
    min_pass_rate: 0.0,
};

impl ProbabilisticTest {
    /// Begins a contract-driven probabilistic test.
    ///
    /// The contract supplies the inputs' element type, the invocation, the
    /// acceptance criteria, and any latency commitment. Set the sampling plan
    /// with [`approach`](ContractTest::approach) (or the
    /// [`samples`](ContractTest::samples) shortcut) and the inputs with
    /// [`inputs`](ContractTest::inputs); execute with [`run`](ContractTest::run).
    #[must_use]
    pub const fn for_contract<C: ServiceContract>(contract: C) -> ContractTest<'static, C> {
        ContractTest::new(contract)
    }
}

/// A contract-driven probabilistic test under construction.
///
/// Builder calls are order-independent; the sampling plan and contract are
/// assembled and validated at [`run`](Self::run).
pub struct ContractTest<'a, C: ServiceContract> {
    contract: C,
    inputs: Option<&'a [C::Input]>,
    approach: ThresholdApproach,
    intent: TestIntent,
    threshold_origin: ThresholdOrigin,
    contract_ref: Option<String>,
    baseline_spec: Option<BaselineSpec>,
    spec_resolver: Option<SpecResolver>,
    baseline_path: Option<PathBuf>,
    baseline_dir: Option<PathBuf>,
    covariate_context: Option<CovariateContext>,
    config_overrides: Option<ExecutionConfig>,
    time_budget: Option<Duration>,
    token_budget: Option<u64>,
    pacing: Option<PacingConfig>,
    on_budget_exhausted: Option<BudgetExhaustedBehavior>,
    baseline_latency_mode: Option<LatencyEnforcementMode>,
    baseline_latency_confidence: Option<f64>,
    fail_on_expired_baseline: bool,
    transparent_stats: bool,
    early_termination_disabled: bool,
}

impl<'a, C: ServiceContract> ContractTest<'a, C> {
    const fn new(contract: C) -> Self {
        Self {
            contract,
            inputs: None,
            approach: DEFAULT_APPROACH,
            intent: TestIntent::Verification,
            threshold_origin: ThresholdOrigin::Empirical,
            contract_ref: None,
            baseline_spec: None,
            spec_resolver: None,
            baseline_path: None,
            baseline_dir: None,
            covariate_context: None,
            config_overrides: None,
            time_budget: None,
            token_budget: None,
            pacing: None,
            on_budget_exhausted: None,
            baseline_latency_mode: None,
            baseline_latency_confidence: None,
            fail_on_expired_baseline: false,
            transparent_stats: false,
            early_termination_disabled: false,
        }
    }

    /// Sets the inputs the contract is invoked against. Inputs are cycled
    /// round-robin when the sample count exceeds the input count.
    #[must_use]
    pub const fn inputs(mut self, inputs: &'a [C::Input]) -> Self {
        self.inputs = Some(inputs);
        self
    }

    /// Sets the sampling plan and how the success-rate threshold is derived.
    #[must_use]
    pub const fn approach(mut self, approach: ThresholdApproach) -> Self {
        self.approach = approach;
        self
    }

    /// Shortcut for a sample count under a threshold-first plan with a `0.0`
    /// floor — each criterion's own target still decides its verdict.
    #[must_use]
    pub const fn samples(mut self, samples: u32) -> Self {
        self.approach = ThresholdApproach::ThresholdFirst {
            samples,
            min_pass_rate: 0.0,
        };
        self
    }

    /// Marks this as a smoke test rather than a verification.
    #[must_use]
    pub const fn smoke(mut self) -> Self {
        self.intent = TestIntent::Smoke;
        self
    }

    /// Records the origin of the criteria's targets (defaults to empirical).
    #[must_use]
    pub const fn threshold_origin(mut self, origin: ThresholdOrigin) -> Self {
        self.threshold_origin = origin;
        self
    }

    /// Sets a human-readable contract reference for provenance.
    #[must_use]
    // mavai-ref: JVI-GQXC6W9 — do not remove (resolves in mavai-orchestrator)
    pub fn contract_ref(mut self, contract_ref: impl Into<String>) -> Self {
        self.contract_ref = Some(contract_ref.into());
        self
    }

    /// Supplies a pre-loaded baseline an empirical criterion derives from.
    #[must_use]
    pub fn baseline_spec(mut self, baseline: BaselineSpec) -> Self {
        self.baseline_spec = Some(baseline);
        self
    }

    /// Sets a spec resolver for loading the baseline by service-contract id.
    #[must_use]
    pub fn spec_resolver(mut self, resolver: SpecResolver) -> Self {
        self.spec_resolver = Some(resolver);
        self
    }

    /// Sets the directory the default spec resolver loads baselines from.
    #[must_use]
    pub fn baseline_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.baseline_dir = Some(dir.into());
        self
    }

    /// Sets an explicit baseline spec file path.
    #[must_use]
    pub fn baseline_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.baseline_path = Some(path.into());
        self
    }

    /// Supplies a covariate context for covariate-aware baseline selection.
    #[must_use]
    pub fn covariate_context(mut self, context: CovariateContext) -> Self {
        self.covariate_context = Some(context);
        self
    }

    /// Sets an explicit execution config, bypassing the synthesised one.
    #[must_use]
    pub const fn execution_config(mut self, config: ExecutionConfig) -> Self {
        self.config_overrides = Some(config);
        self
    }

    /// Sets a wall-clock time budget.
    #[must_use]
    pub const fn time_budget(mut self, budget: Duration) -> Self {
        self.time_budget = Some(budget);
        self
    }

    /// Sets a token budget.
    #[must_use]
    pub const fn token_budget(mut self, budget: u64) -> Self {
        self.token_budget = Some(budget);
        self
    }

    /// Sets pacing constraints.
    #[must_use]
    pub const fn pacing(mut self, pacing: PacingConfig) -> Self {
        self.pacing = Some(pacing);
        self
    }

    /// Sets what happens when a budget is exhausted.
    #[must_use]
    pub const fn on_budget_exhausted(mut self, behaviour: BudgetExhaustedBehavior) -> Self {
        self.on_budget_exhausted = Some(behaviour);
        self
    }

    /// Enforces baseline-derived latency thresholds strictly (fail) rather than
    /// the default advisory (warn).
    #[must_use]
    pub const fn enforce_baseline_latency(mut self, strict: bool) -> Self {
        self.baseline_latency_mode = Some(if strict {
            LatencyEnforcementMode::Strict
        } else {
            LatencyEnforcementMode::Advisory
        });
        self
    }

    /// Overrides the confidence used when deriving baseline latency thresholds.
    #[must_use]
    pub const fn baseline_latency_confidence(mut self, confidence: f64) -> Self {
        self.baseline_latency_confidence = Some(confidence);
        self
    }

    /// Fails the verdict (rather than only warning) when the baseline expired.
    #[must_use]
    pub const fn fail_on_expired_baseline(mut self, fail: bool) -> Self {
        self.fail_on_expired_baseline = fail;
        self
    }

    /// Emits a detailed statistics block to stderr after the run.
    #[must_use]
    pub const fn transparent_stats(mut self, enabled: bool) -> Self {
        self.transparent_stats = enabled;
        self
    }

    /// Disables early termination for this test: every declared sample runs
    /// even once the verdict is statistically determined.
    ///
    /// Early termination (failure-inevitable / success-guaranteed) is on by
    /// default. Disable it when a run needs the full sample set regardless
    /// of the outcome — e.g. to emit a complete latency distribution, capture
    /// every failure exemplar, or feed downstream baseline emission. With the
    /// override active the run reports `TerminationReason::Completed`; the
    /// verdict is unchanged (it depends only on the final pass count).
    #[must_use]
    pub const fn disable_early_termination(mut self) -> Self {
        self.early_termination_disabled = true;
        self
    }

    /// Executes the test and produces the verdict.
    ///
    /// # Panics
    ///
    /// Panics if no inputs were set, if the configuration is infeasible under
    /// verification intent, or if a service invocation yields a defect.
    #[must_use]
    pub fn run(self) -> ProbabilisticTestResult
    where
        C::Output: 'static,
    {
        let inputs = self
            .inputs
            .expect("a probabilistic test requires inputs — call .inputs(...)");

        let spec_resolver = self.resolve_spec_resolver();
        let criteria = AssessmentCriteria {
            approach: self.approach.clone(),
            intent: self.intent,
            threshold_origin: self.threshold_origin,
            contract_ref: self.contract_ref,
            latency: LatencyConfig {
                thresholds: crate::latency::LatencyThresholds::new(),
                baseline_mode: self.baseline_latency_mode,
                baseline_confidence: self
                    .baseline_latency_confidence
                    .unwrap_or(crate::latency::DEFAULT_BASELINE_CONFIDENCE),
            },
            fail_on_expired_baseline: self.fail_on_expired_baseline,
            on_budget_exhausted: self.on_budget_exhausted,
            early_termination_disabled: self.early_termination_disabled,
        };
        // Covariates are part of the contract's identity, so the covariate
        // context is derived from the contract itself unless one was supplied
        // explicitly.
        let covariate_context = self
            .covariate_context
            .or_else(|| CovariateContext::from_contract(&self.contract));
        let baseline = BaselineContext {
            spec_resolver,
            pre_resolved_spec: self.baseline_spec,
            covariate_context,
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

        let result = runner::execute_contract(
            &self.contract,
            inputs,
            &criteria,
            baseline,
            config_overrides.as_ref(),
        );

        crate::reporting::ConsoleRenderer::new().print_verdict(result.verdict_record());
        if self.transparent_stats {
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

    /// Determines the effective spec resolver: an explicit one wins; otherwise
    /// one is synthesised when the plan needs a baseline (non-threshold-first
    /// approach, declared covariates, or an explicit path/dir).
    fn resolve_spec_resolver(&self) -> Option<SpecResolver> {
        if let Some(resolver) = &self.spec_resolver {
            return Some(resolver.clone());
        }
        if self.baseline_spec.is_some() {
            return None;
        }
        let needs_baseline = !matches!(self.approach, ThresholdApproach::ThresholdFirst { .. });
        let has_covariates = self.covariate_context.is_some();
        let explicit_location = self.baseline_path.is_some() || self.baseline_dir.is_some();
        if !needs_baseline && !has_covariates && !explicit_location {
            return None;
        }
        Some(build_default_spec_resolver(
            self.baseline_path.as_deref(),
            self.baseline_dir.as_deref(),
        ))
    }
}
