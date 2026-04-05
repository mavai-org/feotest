//! Proc-macros for the feotest probabilistic testing framework.
//!
//! Provides attribute macros that expand to standard `#[test]` functions
//! wrapping feotest builder invocations:
//!
//! - `#[probabilistic_test]` — probabilistic test with statistical inference

mod expand;
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
