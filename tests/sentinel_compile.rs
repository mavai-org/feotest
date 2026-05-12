//! Compile-time diagnostic tests for the `#[sentinel]` and
//! `#[service_contract_factory]` attribute macros.
//!
//! Each fixture under `tests/ui/sentinel/` exercises one failure mode.
//! The corresponding `.stderr` file captures the expected diagnostic.

#[test]
fn compile_failures() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/sentinel/sentinel_on_enum.rs");
    t.compile_fail("tests/ui/sentinel/service_contract_factory_wrong_return.rs");
    t.compile_fail("tests/ui/sentinel/sentinel_unknown_arg.rs");
}
