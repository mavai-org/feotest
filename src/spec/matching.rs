//! Covariate value matching for baseline selection.
//!
//! Compares resolved covariate values from a test profile against the values
//! stored in a baseline spec. Each covariate is matched independently; the
//! selector uses the aggregate results to score and rank candidates.

/// Result of comparing a baseline covariate value to a test value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResult {
    /// The values are equivalent.
    Conforms,
    /// The values differ.
    DoesNotConform,
}

/// Conformance detail for a single covariate dimension.
#[derive(Debug, Clone)]
pub struct ConformanceDetail {
    key: String,
    baseline_value: String,
    test_value: String,
    result: MatchResult,
}

impl ConformanceDetail {
    /// Creates a new conformance detail.
    pub(crate) fn new(
        key: impl Into<String>,
        baseline_value: impl Into<String>,
        test_value: impl Into<String>,
        result: MatchResult,
    ) -> Self {
        Self {
            key: key.into(),
            baseline_value: baseline_value.into(),
            test_value: test_value.into(),
            result,
        }
    }

    /// The covariate key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The value from the baseline spec.
    #[must_use]
    pub fn baseline_value(&self) -> &str {
        &self.baseline_value
    }

    /// The value from the test profile.
    #[must_use]
    pub fn test_value(&self) -> &str {
        &self.test_value
    }

    /// Whether the values conform.
    #[must_use]
    pub fn conforms(&self) -> bool {
        self.result == MatchResult::Conforms
    }

    /// The match result.
    #[must_use]
    pub const fn result(&self) -> MatchResult {
        self.result
    }
}

/// Matches a single covariate value from a baseline against the test value.
///
/// The `region` key uses case-insensitive comparison; all others use exact
/// string equality.
pub fn match_covariate(key: &str, baseline_value: &str, test_value: &str) -> MatchResult {
    let matches = if key == "region" {
        baseline_value.eq_ignore_ascii_case(test_value)
    } else {
        baseline_value == test_value
    };

    if matches {
        MatchResult::Conforms
    } else {
        MatchResult::DoesNotConform
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_conforms() {
        assert_eq!(
            match_covariate("day-of-week", "WEEKDAY", "WEEKDAY"),
            MatchResult::Conforms,
        );
    }

    #[test]
    fn different_values_do_not_conform() {
        assert_eq!(
            match_covariate("day-of-week", "WEEKDAY", "WEEKEND"),
            MatchResult::DoesNotConform,
        );
    }

    #[test]
    fn case_mismatch_does_not_conform_for_non_region() {
        assert_eq!(
            match_covariate("day-of-week", "weekday", "WEEKDAY"),
            MatchResult::DoesNotConform,
        );
    }

    #[test]
    fn region_is_case_insensitive() {
        assert_eq!(
            match_covariate("region", "eu", "EU"),
            MatchResult::Conforms,
        );
        assert_eq!(
            match_covariate("region", "US", "us"),
            MatchResult::Conforms,
        );
    }

    #[test]
    fn region_mismatch_does_not_conform() {
        assert_eq!(
            match_covariate("region", "EU", "US"),
            MatchResult::DoesNotConform,
        );
    }

    #[test]
    fn empty_strings_conform() {
        assert_eq!(
            match_covariate("key", "", ""),
            MatchResult::Conforms,
        );
    }

    #[test]
    fn conformance_detail_accessors() {
        let detail = ConformanceDetail::new("region", "EU", "US", MatchResult::DoesNotConform);
        assert_eq!(detail.key(), "region");
        assert_eq!(detail.baseline_value(), "EU");
        assert_eq!(detail.test_value(), "US");
        assert!(!detail.conforms());
    }
}
