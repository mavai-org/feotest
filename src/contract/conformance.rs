//! Instance conformance checking: matching actual service responses against
//! known-correct expected outputs.
//!
//! This module provides the matching infrastructure for golden dataset testing,
//! where each test input has a predetermined expected output. Matchers compare
//! actual responses against expected values using pluggable strategies.
//!
//! Conformance checking is a third diagnostic dimension alongside postconditions
//! (correctness) and duration constraints (timing).

use std::fmt;

/// The result of comparing an expected value against an actual value.
///
/// Use the factory methods [`MatchResult::matched`] and [`MatchResult::mismatch`]
/// to create instances.
///
/// # Examples
///
/// ```
/// use feotest::contract::conformance::MatchResult;
///
/// let pass = MatchResult::matched();
/// assert!(pass.is_match());
/// assert_eq!(pass.diff(), "");
///
/// let fail = MatchResult::mismatch("expected 42, got 43");
/// assert!(fail.is_mismatch());
/// assert_eq!(fail.diff(), "expected 42, got 43");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchResult {
    matched: bool,
    diff: String,
}

impl MatchResult {
    /// Creates a successful match result.
    #[must_use]
    pub const fn matched() -> Self {
        Self {
            matched: true,
            diff: String::new(),
        }
    }

    /// Creates a mismatch result with a human-readable diff description.
    #[must_use]
    pub fn mismatch(diff: impl Into<String>) -> Self {
        Self {
            matched: false,
            diff: diff.into(),
        }
    }

    /// Returns `true` if the values matched.
    #[must_use]
    pub const fn is_match(&self) -> bool {
        self.matched
    }

    /// Returns `true` if the values did not match.
    #[must_use]
    pub const fn is_mismatch(&self) -> bool {
        !self.matched
    }

    /// Returns the diff description. Empty string on match.
    #[must_use]
    pub fn diff(&self) -> &str {
        &self.diff
    }
}

impl fmt::Display for MatchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.matched {
            write!(f, "matched")
        } else {
            write!(f, "mismatch: {}", self.diff)
        }
    }
}

/// Compares an expected value against an actual value using a pluggable strategy.
///
/// Matchers are stateless and immutable. A trait is used rather than a closure
/// type alias because matchers carry immutable configuration (e.g.,
/// [`StringMatcher`] holds a comparison mode) and a trait provides a named type
/// for documentation, error messages, and trait object storage.
///
/// # Examples
///
/// ```
/// use feotest::contract::conformance::{VerificationMatcher, MatchResult};
///
/// struct AlwaysMatch;
///
/// impl VerificationMatcher<str> for AlwaysMatch {
///     fn verify(&self, _expected: &str, _actual: &str) -> MatchResult {
///         MatchResult::matched()
///     }
/// }
///
/// let matcher = AlwaysMatch;
/// assert!(matcher.verify("a", "b").is_match());
/// ```
pub trait VerificationMatcher<T: ?Sized> {
    /// Compares `expected` against `actual` and returns a [`MatchResult`].
    fn verify(&self, expected: &T, actual: &T) -> MatchResult;
}

// ---------------------------------------------------------------------------
// StringMatcher
// ---------------------------------------------------------------------------

/// Comparison mode for string matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringMatchMode {
    Exact,
    IgnoreCase,
    TrimWhitespace,
    NormalizeWhitespace,
}

/// Compares strings using a configurable comparison mode.
///
/// All modes are stateless: the mode is set at construction and never changes.
///
/// # Examples
///
/// ```
/// use feotest::contract::conformance::{StringMatcher, VerificationMatcher};
///
/// let matcher = StringMatcher::exact();
/// assert!(matcher.verify("hello", "hello").is_match());
/// assert!(matcher.verify("hello", "Hello").is_mismatch());
///
/// let matcher = StringMatcher::ignore_case();
/// assert!(matcher.verify("hello", "Hello").is_match());
/// ```
#[derive(Debug, Clone)]
pub struct StringMatcher {
    mode: StringMatchMode,
}

impl StringMatcher {
    /// Exact byte-for-byte equality.
    #[must_use]
    pub const fn exact() -> Self {
        Self {
            mode: StringMatchMode::Exact,
        }
    }

    /// Case-insensitive comparison (Unicode-aware via `to_lowercase()`).
    #[must_use]
    pub const fn ignore_case() -> Self {
        Self {
            mode: StringMatchMode::IgnoreCase,
        }
    }

    /// Strips leading and trailing whitespace before comparing.
    #[must_use]
    pub const fn trim_whitespace() -> Self {
        Self {
            mode: StringMatchMode::TrimWhitespace,
        }
    }

    /// Strips leading/trailing whitespace and collapses internal whitespace
    /// runs to single spaces before comparing.
    #[must_use]
    pub const fn normalize_whitespace() -> Self {
        Self {
            mode: StringMatchMode::NormalizeWhitespace,
        }
    }
}

impl VerificationMatcher<str> for StringMatcher {
    fn verify(&self, expected: &str, actual: &str) -> MatchResult {
        match self.mode {
            StringMatchMode::Exact => {
                if expected == actual {
                    MatchResult::matched()
                } else {
                    MatchResult::mismatch(format_diff(expected, actual))
                }
            }
            StringMatchMode::IgnoreCase => {
                let e = expected.to_lowercase();
                let a = actual.to_lowercase();
                if e == a {
                    MatchResult::matched()
                } else {
                    MatchResult::mismatch(format_diff(&e, &a))
                }
            }
            StringMatchMode::TrimWhitespace => {
                let e = expected.trim();
                let a = actual.trim();
                if e == a {
                    MatchResult::matched()
                } else {
                    MatchResult::mismatch(format_diff(e, a))
                }
            }
            StringMatchMode::NormalizeWhitespace => {
                let e = normalize_whitespace(expected);
                let a = normalize_whitespace(actual);
                if e == a {
                    MatchResult::matched()
                } else {
                    MatchResult::mismatch(format_diff(&e, &a))
                }
            }
        }
    }
}

/// Blanket implementation: any matcher for `str` also works for `String`.
impl<M: VerificationMatcher<str>> VerificationMatcher<String> for M {
    fn verify(&self, expected: &String, actual: &String) -> MatchResult {
        VerificationMatcher::<str>::verify(self, expected.as_str(), actual.as_str())
    }
}

// ---------------------------------------------------------------------------
// ConformanceResult
// ---------------------------------------------------------------------------

/// Records the result of an instance conformance check for a single trial.
///
/// Stored on [`super::UseCaseOutcome`] as an optional third diagnostic
/// dimension alongside postcondition and duration results.
#[derive(Debug, Clone)]
pub struct ConformanceResult {
    expected_repr: String,
    match_result: MatchResult,
}

impl ConformanceResult {
    /// Creates a new conformance result.
    #[must_use]
    pub fn new(expected_repr: impl Into<String>, match_result: MatchResult) -> Self {
        Self {
            expected_repr: expected_repr.into(),
            match_result,
        }
    }

    /// Human-readable representation of the expected value.
    #[must_use]
    pub fn expected_repr(&self) -> &str {
        &self.expected_repr
    }

    /// The match result.
    #[must_use]
    pub const fn match_result(&self) -> &MatchResult {
        &self.match_result
    }

    /// Whether the actual value matched the expected value.
    #[must_use]
    pub const fn is_match(&self) -> bool {
        self.match_result.is_match()
    }
}

impl fmt::Display for ConformanceResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.match_result.is_match() {
            write!(f, "conforms to expected: {}", self.expected_repr)
        } else {
            write!(f, "mismatch: {}", self.match_result.diff())
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Maximum length for values in diff messages before truncation.
const MAX_DIFF_LEN: usize = 100;

/// Formats a human-readable diff, truncating long values.
fn format_diff(expected: &str, actual: &str) -> String {
    let e = truncate(expected, MAX_DIFF_LEN);
    let a = truncate(actual, MAX_DIFF_LEN);
    format!("expected: \"{e}\", actual: \"{a}\"")
}

/// Truncates a string at a char boundary, appending `...` if truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        // Find the last char boundary at or before `max`.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Collapses whitespace: trim + replace runs with single space.
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- MatchResult tests ---

    #[test]
    fn match_result_matched_factory() {
        let r = MatchResult::matched();
        assert!(r.is_match());
        assert!(!r.is_mismatch());
        assert_eq!(r.diff(), "");
    }

    #[test]
    fn match_result_mismatch_factory() {
        let r = MatchResult::mismatch("difference");
        assert!(!r.is_match());
        assert!(r.is_mismatch());
        assert_eq!(r.diff(), "difference");
    }

    #[test]
    fn match_result_display_matched() {
        assert_eq!(MatchResult::matched().to_string(), "matched");
    }

    #[test]
    fn match_result_display_mismatch() {
        assert_eq!(
            MatchResult::mismatch("x != y").to_string(),
            "mismatch: x != y"
        );
    }

    #[test]
    fn match_result_equality() {
        assert_eq!(MatchResult::matched(), MatchResult::matched());
        assert_ne!(MatchResult::matched(), MatchResult::mismatch("diff"));
        assert_eq!(
            MatchResult::mismatch("a"),
            MatchResult::mismatch("a".to_string())
        );
    }

    // --- StringMatcher::exact tests ---

    #[test]
    fn exact_matches_equal_strings() {
        let m = StringMatcher::exact();
        assert!(m.verify("hello", "hello").is_match());
    }

    #[test]
    fn exact_mismatches_different_strings() {
        let m = StringMatcher::exact();
        let r = m.verify("hello", "world");
        assert!(r.is_mismatch());
        assert!(r.diff().contains("hello"));
        assert!(r.diff().contains("world"));
    }

    #[test]
    fn exact_matches_empty_strings() {
        let m = StringMatcher::exact();
        assert!(m.verify("", "").is_match());
    }

    #[test]
    fn exact_case_sensitive() {
        let m = StringMatcher::exact();
        assert!(m.verify("Hello", "hello").is_mismatch());
    }

    // --- StringMatcher::ignore_case tests ---

    #[test]
    fn ignore_case_matches_different_cases() {
        let m = StringMatcher::ignore_case();
        assert!(m.verify("Hello World", "hello world").is_match());
    }

    #[test]
    fn ignore_case_mismatches_different_content() {
        let m = StringMatcher::ignore_case();
        assert!(m.verify("hello", "world").is_mismatch());
    }

    #[test]
    fn ignore_case_unicode() {
        let m = StringMatcher::ignore_case();
        assert!(m.verify("Straße", "straße").is_match());
    }

    // --- StringMatcher::trim_whitespace tests ---

    #[test]
    fn trim_whitespace_strips_leading_trailing() {
        let m = StringMatcher::trim_whitespace();
        assert!(m.verify("  hello  ", "hello").is_match());
    }

    #[test]
    fn trim_whitespace_preserves_internal() {
        let m = StringMatcher::trim_whitespace();
        assert!(m.verify("hello  world", "hello world").is_mismatch());
    }

    #[test]
    fn trim_whitespace_handles_tabs_and_newlines() {
        let m = StringMatcher::trim_whitespace();
        assert!(m.verify("\thello\n", "hello").is_match());
    }

    // --- StringMatcher::normalize_whitespace tests ---

    #[test]
    fn normalize_collapses_internal_whitespace() {
        let m = StringMatcher::normalize_whitespace();
        assert!(m.verify("hello   world", "hello world").is_match());
    }

    #[test]
    fn normalize_strips_and_collapses() {
        let m = StringMatcher::normalize_whitespace();
        assert!(m.verify("  hello \t world  ", "hello world").is_match());
    }

    #[test]
    fn normalize_mismatches_different_content() {
        let m = StringMatcher::normalize_whitespace();
        assert!(m.verify("hello world", "goodbye world").is_mismatch());
    }

    // --- Diff truncation tests ---

    #[test]
    fn diff_truncates_long_strings() {
        let long = "a".repeat(200);
        let diff = format_diff(&long, "short");
        assert!(diff.contains("..."));
        assert!(diff.len() < 250);
    }

    #[test]
    fn diff_preserves_short_strings() {
        let diff = format_diff("hello", "world");
        assert!(!diff.contains("..."));
    }

    // --- Custom matcher tests ---

    #[test]
    fn custom_matcher_via_trait_impl() {
        struct LengthMatcher;

        impl VerificationMatcher<str> for LengthMatcher {
            fn verify(&self, expected: &str, actual: &str) -> MatchResult {
                if expected.len() == actual.len() {
                    MatchResult::matched()
                } else {
                    MatchResult::mismatch(format!("length {} != {}", expected.len(), actual.len()))
                }
            }
        }

        let m = LengthMatcher;
        assert!(m.verify("abc", "xyz").is_match());
        assert!(m.verify("ab", "xyz").is_mismatch());
    }

    // --- ConformanceResult tests ---

    #[test]
    fn conformance_result_match() {
        let cr = ConformanceResult::new("42", MatchResult::matched());
        assert!(cr.is_match());
        assert_eq!(cr.expected_repr(), "42");
    }

    #[test]
    fn conformance_result_mismatch() {
        let cr = ConformanceResult::new("42", MatchResult::mismatch("got 43"));
        assert!(!cr.is_match());
        assert_eq!(cr.match_result().diff(), "got 43");
    }

    #[test]
    fn conformance_result_display_match() {
        let cr = ConformanceResult::new("42", MatchResult::matched());
        assert_eq!(cr.to_string(), "conforms to expected: 42");
    }

    #[test]
    fn conformance_result_display_mismatch() {
        let cr = ConformanceResult::new("42", MatchResult::mismatch("got 43"));
        assert_eq!(cr.to_string(), "mismatch: got 43");
    }
}
