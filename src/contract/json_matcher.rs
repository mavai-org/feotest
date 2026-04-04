//! JSON structural matching for instance conformance checking.
//!
//! Compares JSON values semantically, ignoring property ordering and
//! insignificant whitespace. Requires the `json-matcher` feature.

use serde_json::Value;

use super::conformance::{MatchResult, VerificationMatcher};

/// Compares JSON strings semantically, ignoring property ordering and
/// insignificant whitespace.
///
/// # Examples
///
/// ```
/// use feotest::contract::conformance::VerificationMatcher;
/// use feotest::contract::json_matcher::JsonMatcher;
///
/// let matcher = JsonMatcher::new();
/// let result = matcher.verify(
///     r#"{"name":"Alice","age":30}"#,
///     r#"{"age":30,"name":"Alice"}"#,
/// );
/// assert!(result.is_match());
/// ```
#[derive(Debug, Clone)]
pub struct JsonMatcher;

impl JsonMatcher {
    /// Creates a new JSON matcher.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for JsonMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl VerificationMatcher<str> for JsonMatcher {
    fn verify(&self, expected: &str, actual: &str) -> MatchResult {
        let expected_val: Value = match serde_json::from_str(expected) {
            Ok(v) => v,
            Err(e) => return MatchResult::mismatch(format!("invalid expected JSON: {e}")),
        };
        let actual_val: Value = match serde_json::from_str(actual) {
            Ok(v) => v,
            Err(e) => return MatchResult::mismatch(format!("invalid actual JSON: {e}")),
        };

        if expected_val == actual_val {
            MatchResult::matched()
        } else {
            let diff = json_diff("", &expected_val, &actual_val);
            MatchResult::mismatch(diff)
        }
    }
}

/// Produces a human-readable diff between two JSON values.
fn json_diff(path: &str, expected: &Value, actual: &Value) -> String {
    let mut diffs = Vec::new();
    collect_diffs(path, expected, actual, &mut diffs);
    diffs.join("; ")
}

fn collect_diffs(path: &str, expected: &Value, actual: &Value, diffs: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(e_map), Value::Object(a_map)) => {
            for (key, e_val) in e_map {
                let child_path = if path.is_empty() {
                    format!("/{key}")
                } else {
                    format!("{path}/{key}")
                };
                match a_map.get(key) {
                    Some(a_val) => collect_diffs(&child_path, e_val, a_val, diffs),
                    None => diffs.push(format!("at {child_path}: missing in actual")),
                }
            }
            for key in a_map.keys() {
                if !e_map.contains_key(key) {
                    let child_path = if path.is_empty() {
                        format!("/{key}")
                    } else {
                        format!("{path}/{key}")
                    };
                    diffs.push(format!("at {child_path}: unexpected in actual"));
                }
            }
        }
        (Value::Array(e_arr), Value::Array(a_arr)) => {
            let max_len = e_arr.len().max(a_arr.len());
            for i in 0..max_len {
                let child_path = if path.is_empty() {
                    format!("/[{i}]")
                } else {
                    format!("{path}/[{i}]")
                };
                match (e_arr.get(i), a_arr.get(i)) {
                    (Some(e_val), Some(a_val)) => {
                        collect_diffs(&child_path, e_val, a_val, diffs);
                    }
                    (Some(_), None) => diffs.push(format!("at {child_path}: missing in actual")),
                    (None, Some(_)) => diffs.push(format!("at {child_path}: unexpected in actual")),
                    (None, None) => {}
                }
            }
        }
        _ => {
            if expected != actual {
                let loc = if path.is_empty() { "/" } else { path };
                diffs.push(format!("at {loc}: expected {expected}, actual {actual}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_identical_json() {
        let m = JsonMatcher::new();
        assert!(m.verify(r#"{"a":1}"#, r#"{"a":1}"#).is_match());
    }

    #[test]
    fn matches_reordered_properties() {
        let m = JsonMatcher::new();
        let result = m.verify(
            r#"{"name":"Alice","age":30}"#,
            r#"{"age":30,"name":"Alice"}"#,
        );
        assert!(result.is_match());
    }

    #[test]
    fn mismatches_different_values() {
        let m = JsonMatcher::new();
        let result = m.verify(r#"{"name":"Alice"}"#, r#"{"name":"Bob"}"#);
        assert!(result.is_mismatch());
        assert!(result.diff().contains("/name"));
    }

    #[test]
    fn mismatches_missing_key() {
        let m = JsonMatcher::new();
        let result = m.verify(r#"{"a":1,"b":2}"#, r#"{"a":1}"#);
        assert!(result.is_mismatch());
        assert!(result.diff().contains("/b"));
        assert!(result.diff().contains("missing"));
    }

    #[test]
    fn mismatches_extra_key() {
        let m = JsonMatcher::new();
        let result = m.verify(r#"{"a":1}"#, r#"{"a":1,"b":2}"#);
        assert!(result.is_mismatch());
        assert!(result.diff().contains("/b"));
        assert!(result.diff().contains("unexpected"));
    }

    #[test]
    fn reports_invalid_expected_json() {
        let m = JsonMatcher::new();
        let result = m.verify("not json", r#"{"a":1}"#);
        assert!(result.is_mismatch());
        assert!(result.diff().contains("invalid expected JSON"));
    }

    #[test]
    fn reports_invalid_actual_json() {
        let m = JsonMatcher::new();
        let result = m.verify(r#"{"a":1}"#, "not json");
        assert!(result.is_mismatch());
        assert!(result.diff().contains("invalid actual JSON"));
    }

    #[test]
    fn matches_nested_objects() {
        let m = JsonMatcher::new();
        let result = m.verify(r#"{"outer":{"inner":42}}"#, r#"{"outer":{"inner":42}}"#);
        assert!(result.is_match());
    }

    #[test]
    fn mismatches_nested_values() {
        let m = JsonMatcher::new();
        let result = m.verify(r#"{"outer":{"inner":42}}"#, r#"{"outer":{"inner":99}}"#);
        assert!(result.is_mismatch());
        assert!(result.diff().contains("/outer/inner"));
    }

    #[test]
    fn matches_arrays() {
        let m = JsonMatcher::new();
        assert!(m.verify("[1,2,3]", "[1,2,3]").is_match());
    }

    #[test]
    fn mismatches_arrays_different_length() {
        let m = JsonMatcher::new();
        let result = m.verify("[1,2,3]", "[1,2]");
        assert!(result.is_mismatch());
        assert!(result.diff().contains("missing"));
    }

    #[test]
    fn matches_simple_values() {
        let m = JsonMatcher::new();
        assert!(m.verify("42", "42").is_match());
        assert!(m.verify(r#""hello""#, r#""hello""#).is_match());
        assert!(m.verify("true", "true").is_match());
        assert!(m.verify("null", "null").is_match());
    }
}
