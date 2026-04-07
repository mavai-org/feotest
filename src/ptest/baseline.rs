//! Baseline spec resolution for probabilistic tests.
//!
//! Resolves baseline specs from the filesystem, with optional
//! covariate-aware selection. Enforces integrity verification —
//! a tampered spec is a hard failure.

use crate::model::Warning;
use crate::spec::{BaselineSpec, SpecResolver};
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
        return match resolver.resolve(use_case_id) {
            Ok(spec) => Some(spec),
            Err(e) => {
                panic_on_integrity_error(&e);
                warnings.push(Warning::new("BASELINE_SELECTION_FAILED", e.to_string()));
                None
            }
        };
    };

    match resolver.resolve_with_covariates(use_case_id, ctx.profile(), ctx.declarations()) {
        Ok(result) => {
            for detail in result.non_conforming() {
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
            if result.ambiguous() {
                warnings.push(Warning::new(
                    "AMBIGUOUS_BASELINE",
                    format!(
                        "multiple equally-scored baselines ({} candidates)",
                        result.candidate_count(),
                    ),
                ));
            }
            Some(result.into_selected())
        }
        Err(e) => {
            panic_on_integrity_error_resolve(&e);
            warnings.push(Warning::new("BASELINE_SELECTION_FAILED", e.to_string()));
            None
        }
    }
}

/// Panics if the error represents an integrity failure.
///
/// Non-integrity errors (spec not found, selection mismatch) are legitimate
/// runtime conditions that the caller can handle. An integrity failure means
/// a spec file has been tampered with — the test must not proceed.
fn panic_on_integrity_error_resolve(e: &crate::spec::SpecResolveError) {
    if let crate::spec::SpecResolveError::Integrity { path, source } = e {
        if matches!(
            source,
            crate::spec::SpecLoadError::MissingFingerprint { .. }
                | crate::spec::SpecLoadError::IntegrityFailure { .. }
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

/// Panics if the error represents an integrity failure (non-covariate path).
fn panic_on_integrity_error(e: &crate::spec::SpecResolveError) {
    panic_on_integrity_error_resolve(e);
}
