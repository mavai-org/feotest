//! Baseline resolution chain for sentinel-run probabilistic tests.
//!
//! For each probabilistic test whose configuration requires an external
//! baseline specification (EMPIRICAL origin), the sentinel resolves the
//! baseline through a fixed priority order:
//!
//! 1. **External source** — the operator-configured read location
//!    ([`baseline_source_from_env`]). Absent means "skip this step".
//! 2. **Embedded default** — a baseline baked into the binary at build
//!    time via [`crate::sentinel::include_baselines!`] or an equivalent
//!    mechanism.
//! 3. **Unresolvable** — [`BaselineResolutionError`] describing which
//!    stage failed. The sentinel runner converts this into a panic; a
//!    probabilistic test that requires a baseline and cannot resolve one
//!    is a misconfiguration, not a survivable runtime condition.

use core::fmt;
use std::path::{Path, PathBuf};

use crate::spec::baseline::BaselineSpec;
use crate::spec::namer::CovariateProfile;
use crate::spec::{SpecResolveError, SpecResolver};

/// The environment variable that test mode reads to locate authoritative
/// baselines.
pub const SOURCE_ENV_VAR: &str = "FEOTEST_BASELINE_SOURCE";

/// The environment variable measure mode writes to.
pub const OUTPUT_ENV_VAR: &str = "FEOTEST_BASELINE_OUTPUT";

/// Returns the configured baseline-source directory, if any.
///
/// Accepts a bare filesystem path or a `file://` URI. Other URI schemes
/// return `None` — the caller then treats the source as unconfigured and
/// falls back to embedded defaults.
#[must_use]
pub fn baseline_source_from_env() -> Option<PathBuf> {
    std::env::var_os(SOURCE_ENV_VAR).and_then(|raw| parse_file_location(raw.to_str()?))
}

/// Returns the configured baseline-output location, if any.
#[must_use]
pub fn baseline_output_from_env() -> Option<String> {
    std::env::var_os(OUTPUT_ENV_VAR).and_then(|raw| raw.into_string().ok())
}

/// Parses a bare path or `file://` URI into an absolute path. Unsupported
/// schemes return `None`.
pub(crate) fn parse_file_location(raw: &str) -> Option<PathBuf> {
    if let Some(rest) = raw.strip_prefix("file://") {
        return Some(PathBuf::from(rest));
    }
    if raw.contains("://") {
        return None;
    }
    Some(PathBuf::from(raw))
}

/// Inputs for one baseline lookup.
pub struct BaselineQuery<'a> {
    /// Stable name of the owning spec (matches `Sentinel::name()`).
    pub spec_name: &'a str,
    /// Stable name of the test method.
    pub method_name: &'a str,
    /// Covariate profile the test is currently exercising.
    pub covariate_profile: &'a CovariateProfile,
    /// Use-case identifier — baseline filenames are keyed by this.
    pub service_contract_id: &'a str,
}

/// Embedded-baseline registry lookup surface.
///
/// Production code uses [`crate::sentinel::embedded::registry`]; tests
/// typically substitute an in-memory fake.
pub trait EmbeddedBaselineLookup {
    /// Returns a baseline matching the query if one is baked in.
    fn lookup(&self, query: &BaselineQuery<'_>) -> Option<BaselineSpec>;
}

/// Resolves one baseline through the external-source → embedded-default
/// → error chain.
///
/// # Errors
///
/// Returns [`BaselineResolutionError::ExternalStoreUnreachable`] when the
/// configured source directory exists but cannot be read, or when
/// individual file reads fail for reasons other than "not found".
///
/// Returns [`BaselineResolutionError::ExternalStoreMalformed`] when a
/// matching file exists in the external source but does not parse as a
/// valid baseline YAML document (missing fingerprint, integrity failure,
/// bad YAML, etc.).
///
/// Returns [`BaselineResolutionError::EmbeddedDefaultMissing`] when neither
/// the external source (if any) nor the embedded registry contains a
/// matching baseline. In a correctly-built sentinel this case should be
/// structurally impossible; its occurrence indicates a build-pipeline
/// failure or a tampered binary.
pub fn resolve_baseline(
    query: &BaselineQuery<'_>,
    external_source: Option<&Path>,
    embedded: &dyn EmbeddedBaselineLookup,
) -> Result<BaselineSpec, BaselineResolutionError> {
    if let Some(dir) = external_source
        && let Some(spec) = lookup_on_disk(dir, query)?
    {
        return Ok(spec);
    }
    if let Some(spec) = embedded.lookup(query) {
        return Ok(spec);
    }
    Err(BaselineResolutionError::EmbeddedDefaultMissing {
        spec_name: query.spec_name.to_owned(),
        method_name: query.method_name.to_owned(),
        covariate_profile: format_profile(query.covariate_profile),
    })
}

/// Looks up a baseline under a `file://`-style directory using the
/// existing [`SpecResolver`] machinery, which handles filename scanning,
/// covariate-aware selection, and fingerprint-verified loading.
fn lookup_on_disk(
    dir: &Path,
    query: &BaselineQuery<'_>,
) -> Result<Option<BaselineSpec>, BaselineResolutionError> {
    let resolver = SpecResolver::with_dir(dir);
    match resolver.resolve(query.service_contract_id) {
        Ok(spec) => Ok(Some(spec)),
        Err(SpecResolveError::NotFound { .. }) => Ok(None),
        Err(SpecResolveError::Integrity { source, .. }) => {
            Err(BaselineResolutionError::ExternalStoreMalformed {
                spec_name: query.spec_name.to_owned(),
                method_name: query.method_name.to_owned(),
                covariate_profile: format_profile(query.covariate_profile),
                detail: source.to_string(),
            })
        }
        Err(SpecResolveError::Selection { source, .. }) => {
            Err(BaselineResolutionError::ExternalStoreUnreachable {
                spec_name: query.spec_name.to_owned(),
                method_name: query.method_name.to_owned(),
                detail: source.to_string(),
            })
        }
    }
}

fn format_profile(profile: &CovariateProfile) -> String {
    if profile.is_empty() {
        return "(empty)".to_owned();
    }
    profile
        .entries()
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Structured failure from the baseline resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineResolutionError {
    /// The external source was configured but could not be accessed.
    ExternalStoreUnreachable {
        spec_name: String,
        method_name: String,
        detail: String,
    },
    /// The external source returned a file that did not parse as a valid
    /// baseline YAML document.
    ExternalStoreMalformed {
        spec_name: String,
        method_name: String,
        covariate_profile: String,
        detail: String,
    },
    /// Neither the external source (if any) nor the embedded registry
    /// yielded a matching baseline.
    EmbeddedDefaultMissing {
        spec_name: String,
        method_name: String,
        covariate_profile: String,
    },
}

impl fmt::Display for BaselineResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExternalStoreUnreachable {
                spec_name,
                method_name,
                detail,
            } => write!(
                f,
                "baseline resolution failed for {spec_name}.{method_name}: \
                 external source unreachable ({detail}). Check {SOURCE_ENV_VAR}."
            ),
            Self::ExternalStoreMalformed {
                spec_name,
                method_name,
                covariate_profile,
                detail,
            } => write!(
                f,
                "baseline resolution failed for {spec_name}.{method_name} \
                 [covariate: {covariate_profile}]: external store returned a \
                 malformed baseline ({detail})"
            ),
            Self::EmbeddedDefaultMissing {
                spec_name,
                method_name,
                covariate_profile,
            } => write!(
                f,
                "baseline resolution failed for {spec_name}.{method_name} \
                 [covariate: {covariate_profile}]: no embedded default \
                 baseline present. This indicates a tampered binary or a \
                 build step that skipped its baseline-invariant check."
            ),
        }
    }
}

impl std::error::Error for BaselineResolutionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::baseline::{
        ExecutionBlock, RequirementsBlock, StatisticsBlock, SuccessRateBlock,
    };
    use std::collections::HashMap;

    fn sample_baseline(service_contract_id: &str) -> BaselineSpec {
        BaselineSpec::new(
            service_contract_id,
            "2026-04-22T00:00:00Z",
            ExecutionBlock {
                samples_planned: 100,
                samples_executed: 100,
                termination_reason: Some("COMPLETED".to_owned()),
            },
            RequirementsBlock {
                min_pass_rate: 0.90,
            },
            StatisticsBlock {
                success_rate: SuccessRateBlock {
                    observed: 0.95,
                    standard_error: 0.02,
                    confidence_interval95: [0.90, 0.98],
                },
                successes: 95,
                failures: 5,
                failure_distribution: None,
                latency_distribution: None,
                per_criterion: None,
            },
        )
    }

    /// Writes a fingerprinted baseline to the given directory using the
    /// same serialisation `SpecResolver` does for production writes.
    fn write_baseline(dir: &Path, spec: &BaselineSpec) {
        SpecResolver::with_dir(dir)
            .write(spec, &[], &CovariateProfile::empty())
            .expect("write fingerprinted baseline");
    }

    struct FakeEmbedded {
        entries: HashMap<String, BaselineSpec>,
    }

    impl FakeEmbedded {
        fn empty() -> Self {
            Self {
                entries: HashMap::new(),
            }
        }

        fn with(spec_name: &str, method_name: &str, baseline: BaselineSpec) -> Self {
            let mut entries = HashMap::new();
            entries.insert(format!("{spec_name}::{method_name}"), baseline);
            Self { entries }
        }
    }

    impl EmbeddedBaselineLookup for FakeEmbedded {
        fn lookup(&self, query: &BaselineQuery<'_>) -> Option<BaselineSpec> {
            self.entries
                .get(&format!("{}::{}", query.spec_name, query.method_name))
                .cloned()
        }
    }

    #[test]
    fn empty_chain_returns_embedded_default_missing() {
        let profile = CovariateProfile::empty();
        let query = BaselineQuery {
            spec_name: "my_spec",
            method_name: "my_test",
            covariate_profile: &profile,
            service_contract_id: "my_spec.my_test",
        };
        let err = resolve_baseline(&query, None, &FakeEmbedded::empty())
            .expect_err("empty chain must fail");
        let BaselineResolutionError::EmbeddedDefaultMissing {
            spec_name,
            method_name,
            ..
        } = err
        else {
            panic!("unexpected error variant");
        };
        assert_eq!(spec_name, "my_spec");
        assert_eq!(method_name, "my_test");
    }

    #[test]
    fn embedded_default_used_when_no_external_source() {
        let profile = CovariateProfile::empty();
        let query = BaselineQuery {
            spec_name: "my_spec",
            method_name: "my_test",
            covariate_profile: &profile,
            service_contract_id: "my_spec.my_test",
        };
        let embedded = FakeEmbedded::with("my_spec", "my_test", sample_baseline("my_spec.my_test"));
        let resolved = resolve_baseline(&query, None, &embedded).expect("embedded hit");
        assert_eq!(resolved.service_contract_id, "my_spec.my_test");
    }

    #[test]
    fn external_source_takes_precedence_over_embedded_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let profile = CovariateProfile::empty();
        let service_contract_id = "primary";
        let external = sample_baseline(service_contract_id);
        write_baseline(tmp.path(), &external);

        let query = BaselineQuery {
            spec_name: "my_spec",
            method_name: "my_test",
            covariate_profile: &profile,
            service_contract_id,
        };
        // Populate embedded with a baseline that carries a *different* use
        // case id so we can assert which one was returned.
        let fallback = sample_baseline("embedded-fallback");
        let embedded = FakeEmbedded::with("my_spec", "my_test", fallback);

        let resolved = resolve_baseline(&query, Some(tmp.path()), &embedded).expect("external hit");
        assert_eq!(
            resolved.service_contract_id, "primary",
            "external source must win over embedded default"
        );
    }

    #[test]
    fn external_malformed_file_surfaces_malformed_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let profile = CovariateProfile::empty();
        let service_contract_id = "malformed_case";
        // Write a file matching the sanitized-name-plus-dash prefix convention
        // SpecResolver uses, but whose YAML does not pass fingerprint verification.
        std::fs::write(
            tmp.path().join("malformed_case-00000000.yaml"),
            "schemaVersion: feotest-spec-1\nuseCaseId: malformed_case\n",
        )
        .expect("write");

        let query = BaselineQuery {
            spec_name: "my_spec",
            method_name: "my_test",
            covariate_profile: &profile,
            service_contract_id,
        };
        let err = resolve_baseline(&query, Some(tmp.path()), &FakeEmbedded::empty())
            .expect_err("malformed YAML must not silently fall through");
        assert!(
            matches!(err, BaselineResolutionError::ExternalStoreMalformed { .. }),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn external_miss_falls_back_to_embedded() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Directory exists but contains nothing.
        let profile = CovariateProfile::empty();
        let service_contract_id = "missing_from_disk";
        let query = BaselineQuery {
            spec_name: "my_spec",
            method_name: "my_test",
            covariate_profile: &profile,
            service_contract_id,
        };
        let embedded =
            FakeEmbedded::with("my_spec", "my_test", sample_baseline("embedded-fallback"));
        let resolved =
            resolve_baseline(&query, Some(tmp.path()), &embedded).expect("embedded fallback");
        assert_eq!(resolved.service_contract_id, "embedded-fallback");
    }

    #[test]
    fn parse_file_location_bare_path() {
        assert_eq!(
            parse_file_location("/tmp/baselines"),
            Some(PathBuf::from("/tmp/baselines"))
        );
    }

    #[test]
    fn parse_file_location_file_scheme() {
        assert_eq!(
            parse_file_location("file:///tmp/baselines"),
            Some(PathBuf::from("/tmp/baselines"))
        );
    }

    #[test]
    fn parse_file_location_unsupported_scheme_returns_none() {
        assert_eq!(parse_file_location("s3://bucket/baselines"), None);
    }

    #[test]
    fn error_display_mentions_stage_and_components() {
        let err = BaselineResolutionError::EmbeddedDefaultMissing {
            spec_name: "payments".to_owned(),
            method_name: "check_pass_rate".to_owned(),
            covariate_profile: "(empty)".to_owned(),
        };
        let s = err.to_string();
        assert!(s.contains("payments"));
        assert!(s.contains("check_pass_rate"));
        assert!(s.contains("(empty)"));
    }
}
