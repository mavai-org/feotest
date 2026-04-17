//! Baseline spec resolution for probabilistic tests.
//!
//! Resolves baseline specs from the filesystem, with optional
//! covariate-aware selection. Enforces integrity verification —
//! a tampered spec is a hard failure.

use crate::model::Warning;
use crate::spec::baseline::SpecLoadError;
use crate::spec::selector::SelectionResult;
use crate::spec::{BaselineSpec, SpecResolveError, SpecResolver};
use crate::usecase::CovariateContext;

/// Resolves a baseline spec, using covariate-aware selection when context is available.
///
/// # Panics
///
/// Panics if a baseline spec fails integrity verification. A tampered or
/// unsigned spec is an environment configuration error — the test must not
/// proceed with compromised data.
pub fn resolve(
    resolver: &SpecResolver,
    use_case_id: &str,
    covariate_context: Option<&CovariateContext>,
    warnings: &mut Vec<Warning>,
) -> Option<BaselineSpec> {
    let Some(ctx) = covariate_context else {
        let result = resolver.resolve(use_case_id);
        return interpret_resolve_result(result, warnings);
    };

    let result = resolver.resolve_with_covariates(use_case_id, ctx.profile(), ctx.declarations());
    interpret_covariate_result(result, warnings)
}

/// Interprets a non-covariate resolve result.
///
/// Panics on integrity errors; pushes a warning and returns `None` on
/// other failures (e.g., spec not found).
fn interpret_resolve_result(
    result: Result<BaselineSpec, SpecResolveError>,
    warnings: &mut Vec<Warning>,
) -> Option<BaselineSpec> {
    match result {
        Ok(spec) => Some(spec),
        Err(e) => {
            check_integrity_error(&e);
            warnings.push(Warning::new("BASELINE_SELECTION_FAILED", e.to_string()));
            None
        }
    }
}

/// Interprets a covariate-aware resolve result.
///
/// On success, inspects the selection for non-conforming covariates and
/// ambiguity, pushing appropriate warnings. On failure, panics on
/// integrity errors and pushes a warning for everything else.
fn interpret_covariate_result(
    result: Result<SelectionResult, SpecResolveError>,
    warnings: &mut Vec<Warning>,
) -> Option<BaselineSpec> {
    match result {
        Ok(selection) => {
            collect_selection_warnings(&selection, warnings);
            Some(selection.into_selected())
        }
        Err(e) => {
            check_integrity_error(&e);
            warnings.push(Warning::new("BASELINE_SELECTION_FAILED", e.to_string()));
            None
        }
    }
}

/// Pushes warnings for covariate mismatches and ambiguous baselines.
fn collect_selection_warnings(selection: &SelectionResult, warnings: &mut Vec<Warning>) {
    for detail in selection.non_conforming() {
        warnings.push(Warning::new(
            "COVARIATE_MISMATCH",
            format!(
                "covariate '{}': baseline='{}', test='{}'",
                detail.key(),
                detail.baseline_value(),
                detail.test_value(),
            ),
        ));
    }
    if selection.ambiguous() {
        warnings.push(Warning::new(
            "AMBIGUOUS_BASELINE",
            format!(
                "multiple equally-scored baselines ({} candidates)",
                selection.candidate_count(),
            ),
        ));
    }
}

/// Panics if the error represents an integrity failure.
///
/// Non-integrity errors (spec not found, selection mismatch) are legitimate
/// runtime conditions that the caller can handle. An integrity failure means
/// a spec file has been tampered with — the test must not proceed.
fn check_integrity_error(e: &SpecResolveError) {
    if let SpecResolveError::Integrity { path, source } = e {
        if matches!(
            source,
            SpecLoadError::MissingFingerprint { .. } | SpecLoadError::IntegrityFailure { .. }
        ) {
            panic!(
                "\n\nBaseline spec integrity check failed.\n\n\
                 File: {}\n\
                 {source}\n",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // -----------------------------------------------------------------------
    // interpret_resolve_result
    // -----------------------------------------------------------------------

    #[test]
    fn interpret_ok_returns_spec() {
        let spec = test_spec();
        let mut warnings = Vec::new();
        let result = interpret_resolve_result(Ok(spec.clone()), &mut warnings);
        assert!(result.is_some());
        assert!(warnings.is_empty());
    }

    #[test]
    fn interpret_not_found_pushes_warning_and_returns_none() {
        let err = SpecResolveError::NotFound {
            use_case_id: "test".to_string(),
            path: PathBuf::from("/fake/path"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
        };
        let mut warnings = Vec::new();
        let result = interpret_resolve_result(Err(err), &mut warnings);
        assert!(result.is_none());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "BASELINE_SELECTION_FAILED");
    }

    #[test]
    #[should_panic(expected = "integrity check failed")]
    fn interpret_integrity_failure_panics() {
        let err = SpecResolveError::Integrity {
            path: PathBuf::from("/fake/spec.yaml"),
            source: SpecLoadError::IntegrityFailure {
                use_case_id: "test".to_string(),
                expected: "aaa".to_string(),
                actual: "bbb".to_string(),
            },
        };
        let mut warnings = Vec::new();
        interpret_resolve_result(Err(err), &mut warnings);
    }

    #[test]
    #[should_panic(expected = "integrity check failed")]
    fn interpret_missing_fingerprint_panics() {
        let err = SpecResolveError::Integrity {
            path: PathBuf::from("/fake/spec.yaml"),
            source: SpecLoadError::MissingFingerprint {
                use_case_id: "test".to_string(),
            },
        };
        let mut warnings = Vec::new();
        interpret_resolve_result(Err(err), &mut warnings);
    }

    // -----------------------------------------------------------------------
    // interpret_covariate_result
    // -----------------------------------------------------------------------

    #[test]
    fn covariate_not_found_pushes_warning() {
        let err = SpecResolveError::NotFound {
            use_case_id: "test".to_string(),
            path: PathBuf::from("/fake"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
        };
        let mut warnings = Vec::new();
        let result = interpret_covariate_result(Err(err), &mut warnings);
        assert!(result.is_none());
        assert_eq!(warnings[0].code(), "BASELINE_SELECTION_FAILED");
    }

    #[test]
    #[should_panic(expected = "integrity check failed")]
    fn covariate_integrity_failure_panics() {
        let err = SpecResolveError::Integrity {
            path: PathBuf::from("/fake/spec.yaml"),
            source: SpecLoadError::IntegrityFailure {
                use_case_id: "test".to_string(),
                expected: "aaa".to_string(),
                actual: "bbb".to_string(),
            },
        };
        let mut warnings = Vec::new();
        interpret_covariate_result(Err(err), &mut warnings);
    }

    // -----------------------------------------------------------------------
    // check_integrity_error
    // -----------------------------------------------------------------------

    #[test]
    fn non_integrity_error_does_not_panic() {
        let err = SpecResolveError::NotFound {
            use_case_id: "test".to_string(),
            path: PathBuf::from("/fake"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
        };
        // Should not panic
        check_integrity_error(&err);
    }

    // -----------------------------------------------------------------------
    // collect_selection_warnings
    // -----------------------------------------------------------------------

    #[test]
    fn no_warnings_when_selection_clean() {
        let selection = SelectionResult::from_single(test_spec());
        let mut warnings = Vec::new();
        collect_selection_warnings(&selection, &mut warnings);
        assert!(warnings.is_empty());
    }

    #[test]
    fn covariate_mismatch_warnings_collected() {
        use crate::spec::matching::{ConformanceDetail, MatchResult};

        let conformance = vec![ConformanceDetail::new(
            "model",
            "gpt-4o",
            "gpt-3.5",
            MatchResult::DoesNotConform,
        )];
        let selection = SelectionResult::with_details(test_spec(), conformance, false, 2);
        let mut warnings = Vec::new();
        collect_selection_warnings(&selection, &mut warnings);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "COVARIATE_MISMATCH");
        assert!(warnings[0].message().contains("model"));
        assert!(warnings[0].message().contains("gpt-4o"));
        assert!(warnings[0].message().contains("gpt-3.5"));
    }

    #[test]
    fn ambiguous_baseline_warning_collected() {
        let selection = SelectionResult::with_details(test_spec(), Vec::new(), true, 3);
        let mut warnings = Vec::new();
        collect_selection_warnings(&selection, &mut warnings);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "AMBIGUOUS_BASELINE");
        assert!(warnings[0].message().contains("3 candidates"));
    }

    #[test]
    fn both_mismatch_and_ambiguity_produce_two_warnings() {
        use crate::spec::matching::{ConformanceDetail, MatchResult};

        let conformance = vec![ConformanceDetail::new(
            "region",
            "us-east-1",
            "eu-west-1",
            MatchResult::DoesNotConform,
        )];
        let selection = SelectionResult::with_details(test_spec(), conformance, true, 4);
        let mut warnings = Vec::new();
        collect_selection_warnings(&selection, &mut warnings);

        assert_eq!(warnings.len(), 2);
        assert_eq!(warnings[0].code(), "COVARIATE_MISMATCH");
        assert_eq!(warnings[1].code(), "AMBIGUOUS_BASELINE");
    }

    #[test]
    fn covariate_ok_returns_spec() {
        let selection = SelectionResult::from_single(test_spec());
        let mut warnings = Vec::new();
        let result = interpret_covariate_result(Ok(selection), &mut warnings);
        assert!(result.is_some());
        assert!(warnings.is_empty());
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn test_spec() -> BaselineSpec {
        use crate::spec::baseline::{
            ExecutionBlock, RequirementsBlock, StatisticsBlock, SuccessRateBlock,
        };
        BaselineSpec::new(
            "test",
            "2026-01-01T00:00:00Z",
            ExecutionBlock {
                samples_planned: 100,
                samples_executed: 100,
                termination_reason: Some("COMPLETED".to_string()),
            },
            RequirementsBlock {
                min_pass_rate: 0.90,
            },
            StatisticsBlock {
                success_rate: SuccessRateBlock {
                    observed: 0.95,
                    standard_error: 0.022,
                    confidence_interval95: [0.907, 0.993],
                },
                successes: 95,
                failures: 5,
                failure_distribution: None,
                latency_distribution: None,
            },
        )
    }
}
