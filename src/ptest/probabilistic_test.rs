//! The contract-driven probabilistic test entry point.
//!
//! [`ProbabilisticTest`] is a marker type whose sole role is to host
//! [`ProbabilisticTest::for_contract`](crate::ptest::ProbabilisticTest::for_contract),
//! which binds a test to a [`ServiceContract`](crate::service_contract::ServiceContract)
//! and returns a [`ContractTest`](crate::ptest::ContractTest) builder.

use std::time::Duration;

use crate::controls::{ExecutionConfig, PacingConfig};
use crate::model::BudgetExhaustedBehavior;
use crate::ptest::builder::ThresholdApproach;

/// Entry point for a contract-driven probabilistic test.
///
/// Construct a test with
/// [`ProbabilisticTest::for_contract`](Self::for_contract).
pub struct ProbabilisticTest;

/// Builds optional execution config overrides from the simplified
/// budget/pacing setters.
pub fn build_config_overrides(
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
        // Confidence-first and risk-driven compute samples at runtime. The
        // runner synthesises its own config in those cases; we cannot
        // pre-compute here.
        ThresholdApproach::ConfidenceFirst { .. } | ThresholdApproach::RiskDriven { .. } => {
            return None;
        }
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
