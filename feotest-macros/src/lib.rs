//! Proc-macros for the feotest probabilistic testing framework.
//!
//! Provides attribute macros that expand to standard `#[test]` functions
//! wrapping feotest builder invocations:
//!
//! - `#[probabilistic_test]` — probabilistic test with statistical inference
//! - `#[measure_experiment]` — baseline measurement experiment

mod expand;
mod measure_expand;
mod measure_parse;
mod parse;

use proc_macro::TokenStream;

/// Marks a function as a probabilistic test.
///
/// The macro detects the operational approach from the combination of
/// attributes and expands to a `#[test]` function that configures and
/// runs a `ProbabilisticTestBuilder`.
///
/// # Approaches
///
/// | Approach | Required attributes |
/// |----------|-------------------|
/// | Threshold-first | `samples` + `threshold` |
/// | Sample-size-first | `samples` + `confidence` + `spec` |
/// | Confidence-first | `confidence` + `min_detectable_effect` + `power` + `spec` |
///
/// # Optional attributes
///
/// - `intent` — `"verification"` (default) or `"smoke"`
/// - `threshold_origin` — `"sla"`, `"slo"`, `"policy"`, `"empirical"`
/// - `contract_ref` — human-readable document reference
/// - `transparent_stats` — `true` to include detailed statistics
/// - `time_budget` — wall-clock cap, e.g. `"30s"`, `"5m"`
/// - `pacing` — rate limit, e.g. `"10/s"`, `"100/m"`
#[proc_macro_attribute]
pub fn probabilistic_test(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Marks a function as a measure experiment.
///
/// Expands to a `#[test]` function that runs the trial function many times,
/// computes statistics, and optionally writes a baseline spec to disk.
///
/// # Required attributes
///
/// - `use_case` — use case identifier (string)
/// - `samples` — number of invocations (integer, >= 1)
///
/// # Optional attributes
///
/// - `inputs` — input values cycled round-robin, e.g. `["a", "b"]`
/// - `spec_dir` — directory for baseline spec output
/// - `experiment_id` — identifier written into spec metadata
/// - `warmup` — warmup invocations before counting begins
/// - `time_budget` — wall-clock cap, e.g. `"10m"`, `"600s"`
/// - `token_budget` — token cap across all samples
/// - `pacing` — rate limit, e.g. `"10/s"`, `"100/m"`
#[proc_macro_attribute]
pub fn measure_experiment(attr: TokenStream, item: TokenStream) -> TokenStream {
    measure_expand::expand(attr.into(), item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}
