//! Compile-time diagnostic test for the reference-matching criterion's
//! exclusivity: a matching terminal may not follow a `satisfies` postcondition.
//!
//! The fixture under `tests/ui/matching/` exercises the failure mode; the
//! corresponding `.stderr` file captures the expected diagnostic.

#[test]
fn matching_terminals_are_unavailable_after_satisfies() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/matching/matching_after_satisfies.rs");
}
