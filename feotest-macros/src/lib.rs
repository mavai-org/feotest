//! Proc-macros for the feotest probabilistic testing framework.
//!
//! Provides attribute macros:
//!
//! - `#[probabilistic_test]` — probabilistic test with statistical inference.
//! - `#[sentinel]` — marks a struct as a reliability specification and
//!   registers it into the sentinel inventory.
//! - `#[service_contract_factory]` — marks a method within a `#[sentinel]` struct
//!   as producing a service contract.

mod expand;
mod include_baselines;
mod parse;
mod sentinel;
mod sentinel_impl;
mod service_contract_factory;

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

/// Marks a struct as a reliability specification.
///
/// The macro emits an `impl ReliabilitySpec` for the struct and registers
/// a `SpecDescriptor` into the sentinel inventory at link time. The struct
/// must implement `Default` (derive or hand-written); the generated
/// constructor calls `Default::default()` to produce instances on demand.
///
/// # Arguments
///
/// - `name = "..."` — override the registration name. Defaults to the
///   snake-cased struct identifier.
/// - `description = "..."` — a one-line human description. Defaults to
///   the empty string.
#[proc_macro_attribute]
pub fn sentinel(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_ts: proc_macro2::TokenStream = item.into();
    if let Err(e) = sentinel::validate_is_struct(&item_ts) {
        return e.to_compile_error().into();
    }
    sentinel::expand(attr.into(), item_ts)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Marks a method as a use-case factory within a `#[sentinel]` struct.
///
/// The method must return `impl ServiceContract` or `Box<dyn ServiceContract>`. Any other
/// return shape is a compile-time error. The method itself is emitted
/// unchanged; the attribute's current role is validation and reservation
/// for future discovery machinery.
#[proc_macro_attribute]
pub fn service_contract_factory(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr: proc_macro2::TokenStream = attr.into();
    service_contract_factory::expand(&attr, item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Applied to the `impl` block of a `#[sentinel]` struct. Processes
/// method-level marker attributes (`#[probabilistic_test]`,
/// `#[measure_experiment]`) and emits the registrations that make the
/// methods invokable through the sentinel runtime.
///
/// Inner markers are stripped; the method bodies are emitted unchanged.
/// The marker attributes are not themselves proc-macros — they exist only
/// as configuration tokens consumed by this outer expansion.
///
/// See the `feotest::sentinel` module docs for the full authoring shape.
#[proc_macro_attribute]
pub fn sentinel_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr: proc_macro2::TokenStream = attr.into();
    sentinel_impl::expand(&attr, item.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Embeds a directory of baseline YAML files into the binary so the
/// sentinel's baseline resolver can use them as defaults.
///
/// The argument is a path relative to the invoking crate's
/// `Cargo.toml`. The directory layout is:
///
/// ```text
/// <dir>/<spec-name>/<method-name>.yaml
/// ```
///
/// Each YAML file is embedded verbatim; its filename (without `.yaml`)
/// becomes the `method_name` and its parent directory name becomes the
/// `spec_name` of the registered [`EmbeddedBaseline`] entry.
///
/// Invoke once per sentinel binary — typically from `main.rs` or the
/// crate root.
#[proc_macro]
pub fn include_baselines(input: TokenStream) -> TokenStream {
    include_baselines::expand(input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}
