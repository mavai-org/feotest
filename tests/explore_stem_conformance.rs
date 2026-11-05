//! The mavai-R exploration-stem conformance case
//! (`tests/conformance/interchange/naming-explore-stem.yaml`, vendored
//! from the interchange archive of the release in `tests/conformance/VERSION`).
//!
//! feotest writes each configuration's `Display` name as its stem rather
//! than deriving the stem from factor values, so it asserts the case's
//! `bound` part: the whole-stem bound applied to a given stem (amendment
//! 2026-11-04). The `stems` part, which derives the stem from
//! ordered factors, is owed when feotest adopts the factor-value stem.

use std::path::Path;

use feotest::spec::keys::{MAX_STEM_BYTES, bounded_stem};
use serde_yaml::Value;

fn conformance_case() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/conformance/interchange/naming-explore-stem.yaml");
    serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_case_bound_matches_the_writer() {
    let case = conformance_case();
    assert_eq!(
        case["maxBytes"].as_u64(),
        u64::try_from(MAX_STEM_BYTES).ok()
    );
    assert_eq!(case["hashChars"].as_u64(), Some(8));
}

#[test]
fn bounded_stem_gives_every_expected_stem() {
    let case = conformance_case();
    let entries = case["bound"].as_sequence().unwrap();
    assert!(!entries.is_empty());
    for entry in entries {
        let id = entry["id"].as_str().unwrap();
        let expected = entry["expected"].as_str().unwrap();
        assert_eq!(
            bounded_stem(entry["stem"].as_str().unwrap()),
            expected,
            "{id}"
        );
        assert!(expected.len() <= MAX_STEM_BYTES, "{id}");
    }
}
