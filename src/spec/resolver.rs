//! Spec resolution: finding the right baseline for a service contract.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::service_contract::CovariateDeclaration;
use crate::spec::BaselineSpec;
use crate::spec::baseline::SpecLoadError;
use crate::spec::namer::{CovariateProfile, baseline_filename, compute_footprint};
use crate::spec::selector::{BaselineCandidate, SelectionError, SelectionResult};

/// Resolves baseline specs from the filesystem.
///
/// Searches for specs by service contract ID, checking an environment-override
/// directory first, then the configured default.
#[derive(Debug, Clone)]
pub struct SpecResolver {
    spec_dir: PathBuf,
}

impl SpecResolver {
    /// Creates a resolver with the given default spec directory.
    ///
    /// If the `FEOTEST_SPEC_DIR` environment variable is set, it takes
    /// precedence over the provided default.
    #[must_use]
    pub fn new(default_dir: impl Into<PathBuf>) -> Self {
        let spec_dir =
            std::env::var("FEOTEST_SPEC_DIR").map_or_else(|_| default_dir.into(), PathBuf::from);
        Self { spec_dir }
    }

    /// Creates a resolver with an explicit directory (ignoring env var).
    #[must_use]
    pub fn with_dir(spec_dir: impl Into<PathBuf>) -> Self {
        Self {
            spec_dir: spec_dir.into(),
        }
    }

    /// The directory being searched.
    #[must_use]
    pub fn spec_dir(&self) -> &Path {
        &self.spec_dir
    }

    /// Resolves a baseline spec for the given service contract ID.
    ///
    /// Scans the spec directory for YAML files whose name starts with
    /// the sanitized service contract ID followed by `-`. If multiple candidates
    /// exist, the first match is returned. Use [`resolve_with_covariates`]
    /// for covariate-aware selection.
    ///
    /// # Errors
    ///
    /// Returns an error if no matching file is found or parsing fails.
    pub fn resolve(&self, service_contract_id: &str) -> Result<BaselineSpec, SpecResolveError> {
        let candidates = self.find_candidates(service_contract_id)?;
        candidates
            .into_iter()
            .next()
            .map(|c| c.spec)
            .ok_or_else(|| SpecResolveError::NotFound {
                service_contract_id: service_contract_id.to_string(),
                path: self.spec_dir.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no baseline file for '{service_contract_id}'"),
                ),
            })
    }

    /// Resolves the best baseline spec using covariate-aware selection.
    ///
    /// Scans the spec directory for all candidates matching the service contract ID,
    /// then selects the best match based on the current covariate profile
    /// and declarations.
    ///
    /// # Errors
    ///
    /// Returns an error if no candidates are found, no candidates match the
    /// required configuration covariates, or parsing fails.
    pub fn resolve_with_covariates(
        &self,
        service_contract_id: &str,
        profile: &CovariateProfile,
        declarations: &[CovariateDeclaration],
    ) -> Result<SelectionResult, SpecResolveError> {
        let candidates = self.find_candidates(service_contract_id)?;
        if candidates.is_empty() {
            return Err(SpecResolveError::NotFound {
                service_contract_id: service_contract_id.to_string(),
                path: self.spec_dir.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no baseline file for '{service_contract_id}'"),
                ),
            });
        }
        crate::spec::selector::select(&candidates, profile, declarations).map_err(|e| {
            SpecResolveError::Selection {
                service_contract_id: service_contract_id.to_string(),
                source: e,
            }
        })
    }

    /// Discovers all baseline spec candidates for a service contract.
    ///
    /// Scans the spec directory for YAML files whose name starts with
    /// the sanitized service contract ID followed by `-`.
    pub(crate) fn find_candidates(
        &self,
        service_contract_id: &str,
    ) -> Result<Vec<BaselineCandidate>, SpecResolveError> {
        let sanitized = service_contract_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let prefix = format!("{sanitized}-");

        let entries =
            std::fs::read_dir(&self.spec_dir).map_err(|e| SpecResolveError::NotFound {
                service_contract_id: service_contract_id.to_string(),
                path: self.spec_dir.clone(),
                source: e,
            })?;

        let mut candidates = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&prefix) && name_str.ends_with(".yaml") {
                let path = entry.path();
                let content =
                    std::fs::read_to_string(&path).map_err(|e| SpecResolveError::NotFound {
                        service_contract_id: service_contract_id.to_string(),
                        path: path.clone(),
                        source: e,
                    })?;
                let spec =
                    BaselineSpec::from_yaml(&content).map_err(|e| SpecResolveError::Integrity {
                        path: path.clone(),
                        source: e,
                    })?;
                candidates.push(BaselineCandidate {
                    filename: name_str.into_owned(),
                    spec,
                });
            }
        }
        Ok(candidates)
    }

    /// Reads a baseline spec directly from a file path.
    ///
    /// Use this when the exact file path is known (e.g., from a macro
    /// `spec` attribute) rather than resolving by service contract ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the file is not found or cannot be parsed.
    pub fn resolve_file(path: impl AsRef<Path>) -> Result<BaselineSpec, SpecResolveError> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| SpecResolveError::NotFound {
            service_contract_id: path.display().to_string(),
            path: path.to_path_buf(),
            source: e,
        })?;
        BaselineSpec::from_yaml(&content).map_err(|e| SpecResolveError::Integrity {
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Writes a baseline spec to the spec directory.
    ///
    /// The filename encodes the service contract ID, footprint hash, and covariate
    /// value hashes. See [`crate::spec::namer`] for the filename format.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be written.
    // javai-ref: JVI-EC8CPT3 — do not remove (resolves in javai-orchestrator)
    pub fn write(
        &self,
        spec: &BaselineSpec,
        covariate_keys: &[&str],
        covariate_profile: &CovariateProfile,
    ) -> Result<PathBuf, std::io::Error> {
        std::fs::create_dir_all(&self.spec_dir)?;
        let footprint = compute_footprint(&spec.service_contract_id, covariate_keys);
        let filename = baseline_filename(&spec.service_contract_id, &footprint, covariate_profile);
        let path = self.spec_dir.join(filename);

        // Enrich the spec with footprint, covariates, and content fingerprint
        let mut enriched = spec.clone();
        enriched.footprint = Some(footprint);
        enriched.covariates = covariate_profile
            .entries()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>();

        // Serialize without fingerprint, compute SHA-256, then set it
        enriched.content_fingerprint = None;
        let yaml_without_fp = enriched.to_yaml().map_err(std::io::Error::other)?;
        let digest = Sha256::digest(yaml_without_fp.as_bytes());
        enriched.content_fingerprint = Some(format!("{digest:x}"));

        let yaml = enriched.to_yaml().map_err(std::io::Error::other)?;
        std::fs::write(&path, yaml)?;
        Ok(path)
    }
}

/// Errors that can occur during spec resolution.
#[derive(Debug)]
pub enum SpecResolveError {
    /// The spec file was not found.
    NotFound {
        /// The service contract ID that was looked up.
        service_contract_id: String,
        /// The path that was checked.
        path: PathBuf,
        /// The underlying IO error.
        source: std::io::Error,
    },
    /// The spec file could not be loaded (parse failure, missing fingerprint,
    /// or integrity mismatch).
    Integrity {
        /// The path that was read.
        path: PathBuf,
        /// The underlying load error.
        source: SpecLoadError,
    },
    /// Covariate-aware selection failed (e.g., configuration mismatch).
    Selection {
        /// The service contract ID.
        service_contract_id: String,
        /// The underlying selection error.
        source: SelectionError,
    },
}

impl std::fmt::Display for SpecResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound {
                service_contract_id,
                path,
                ..
            } => write!(
                f,
                "no spec found for service contract '{service_contract_id}' at {}",
                path.display()
            ),
            Self::Integrity { path, source } => {
                write!(f, "spec at {}: {source}", path.display())
            }
            Self::Selection {
                service_contract_id,
                source,
            } => write!(
                f,
                "baseline selection failed for service contract '{service_contract_id}': {source}"
            ),
        }
    }
}

impl std::error::Error for SpecResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotFound { source, .. } => Some(source),
            Self::Integrity { source, .. } => Some(source),
            Self::Selection { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::baseline::{
        ExecutionBlock, RequirementsBlock, StatisticsBlock, SuccessRateBlock,
    };
    use std::fs;

    fn sample_spec() -> BaselineSpec {
        BaselineSpec::new(
            "test-use-case",
            "2026-03-27T10:00:00Z",
            ExecutionBlock {
                samples_planned: 100,
                samples_executed: 100,
                termination_reason: Some("COMPLETED".to_string()),
            },
            RequirementsBlock {
                min_pass_rate: 0.85,
            },
            StatisticsBlock {
                success_rate: SuccessRateBlock {
                    observed: 0.90,
                    standard_error: 0.03,
                    confidence_interval95: [0.85, 0.95],
                },
                successes: 90,
                failures: 10,
                failure_distribution: None,
                latency_distribution: None,
                per_criterion: None,
            },
        )
    }

    #[test]
    fn resolution_is_indifferent_to_the_normative_judgement_marker() {
        use crate::spec::baseline::{
            CriterionStatistics, NormativeJudgementBlock, NormativeJudgementState,
        };
        use std::collections::BTreeMap;

        // Two identical specs, one carrying a normative-judgement marker.
        // The resolver reads both, and everything threshold derivation
        // consumes (successes, failures, per-criterion rates) is unchanged
        // by the marker's presence.
        let with_marker = {
            let mut spec = sample_spec();
            let mut per_criterion = BTreeMap::new();
            per_criterion.insert(
                "content".to_string(),
                CriterionStatistics {
                    success_rate: SuccessRateBlock {
                        observed: 0.90,
                        standard_error: 0.03,
                        confidence_interval95: [0.85, 0.95],
                    },
                    successes: 90,
                    failures: 10,
                    failure_distribution: None,
                    normative_judgement: Some(NormativeJudgementBlock {
                        state: NormativeJudgementState::Failed,
                        stipulated_threshold: 0.99,
                        confidence: 0.95,
                        feasible_minimum_samples: None,
                    }),
                },
            );
            spec.statistics.per_criterion = Some(per_criterion);
            spec
        };

        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());
        resolver
            .write(&with_marker, &[], &CovariateProfile::empty())
            .unwrap();

        let resolved = resolver.resolve("test-use-case").unwrap();
        assert_eq!(resolved.statistics.successes, 90);
        assert_eq!(resolved.statistics.failures, 10);
        let criterion = &resolved.statistics.per_criterion.as_ref().unwrap()["content"];
        assert_eq!(criterion.successes, 90);
        assert_eq!(criterion.failures, 10);
        // The marker is carried through untouched for later readers.
        assert_eq!(
            criterion.normative_judgement.as_ref().unwrap().state,
            NormativeJudgementState::Failed
        );
    }

    #[test]
    fn write_and_resolve_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        let spec = sample_spec();
        let profile = CovariateProfile::empty();
        let path = resolver.write(&spec, &[], &profile).unwrap();
        assert!(path.exists());
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("test-use-case-")
        );

        let loaded_spec = resolver.resolve("test-use-case").unwrap();
        assert_eq!(loaded_spec.service_contract_id, "test-use-case");
        assert!((loaded_spec.requirements.min_pass_rate - 0.85).abs() < 1e-10);
    }

    #[test]
    fn resolve_missing_spec_returns_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        let result = resolver.resolve("nonexistent");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn resolve_malformed_yaml_returns_integrity_error() {
        let dir = tempfile::tempdir().unwrap();
        let spec_path = dir.path().join("bad-spec-broken.yaml");
        fs::write(&spec_path, "not: valid: yaml: [[[").unwrap();

        let resolver = SpecResolver::with_dir(dir.path());
        let result = resolver.resolve("bad-spec");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecResolveError::Integrity { .. }
        ));
    }

    #[test]
    fn find_candidates_returns_all_matching() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        // Write two specs with different covariate profiles
        let spec = sample_spec();
        let profile_eu = CovariateProfile::builder().put("region", "EU").build();
        let profile_us = CovariateProfile::builder().put("region", "US").build();
        resolver.write(&spec, &["region"], &profile_eu).unwrap();
        resolver.write(&spec, &["region"], &profile_us).unwrap();

        let candidates = resolver.find_candidates("test-use-case").unwrap();
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn resolve_with_covariates_selects_best() {
        use crate::service_contract::{CovariateCategory, CovariateDeclaration};

        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        let spec = sample_spec();
        let profile_eu = CovariateProfile::builder().put("region", "EU").build();
        let profile_us = CovariateProfile::builder().put("region", "US").build();
        resolver.write(&spec, &["region"], &profile_eu).unwrap();
        resolver.write(&spec, &["region"], &profile_us).unwrap();

        let test_profile = CovariateProfile::builder().put("region", "US").build();
        let declarations = vec![CovariateDeclaration::new(
            "region",
            CovariateCategory::Infrastructure,
        )];

        let result = resolver
            .resolve_with_covariates("test-use-case", &test_profile, &declarations)
            .unwrap();
        assert_eq!(result.selected().covariates.get("region").unwrap(), "US");
        assert_eq!(result.candidate_count(), 2);
    }

    #[test]
    fn resolve_rejects_tampered_spec() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        // Write a valid spec
        let spec = sample_spec();
        let profile = CovariateProfile::empty();
        let path = resolver.write(&spec, &[], &profile).unwrap();

        // Tamper with it
        let content = fs::read_to_string(&path).unwrap();
        let tampered = content.replace("minPassRate: 0.85", "minPassRate: 0.50");
        fs::write(&path, tampered).unwrap();

        let result = resolver.resolve("test-use-case");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecResolveError::Integrity { .. }
        ));
    }

    #[test]
    fn resolve_file_rejects_tampered_spec() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        let spec = sample_spec();
        let profile = CovariateProfile::empty();
        let path = resolver.write(&spec, &[], &profile).unwrap();

        // Tamper with it
        let content = fs::read_to_string(&path).unwrap();
        let tampered = content.replace("observed: 0.9", "observed: 0.5");
        fs::write(&path, tampered).unwrap();

        let result = SpecResolver::resolve_file(&path);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecResolveError::Integrity { .. }
        ));
    }

    #[test]
    fn spec_dir_returns_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());
        assert_eq!(resolver.spec_dir(), dir.path());
    }

    #[test]
    fn resolve_file_missing_file_returns_not_found() {
        let result = SpecResolver::resolve_file("/nonexistent/path/spec.yaml");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecResolveError::NotFound { .. }
        ));
    }

    #[test]
    fn sanitises_special_characters_in_service_contract_id() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        // Write a spec with a sanitised name: "my.service/v2" → "my_service_v2"
        let mut enriched = sample_spec();
        enriched.service_contract_id = "my.service/v2".to_string();

        // find_candidates sanitises the lookup ID, so the file on disk must match
        // the sanitised prefix. Write it manually with the sanitised name.
        let mut signed = enriched;
        signed.content_fingerprint = None;
        let yaml_without_fp = signed.to_yaml().unwrap();
        let digest = sha2::Sha256::digest(yaml_without_fp.as_bytes());
        signed.content_fingerprint = Some(format!("{digest:x}"));
        let yaml = signed.to_yaml().unwrap();
        let path = dir.path().join("my_service_v2-abcd1234.yaml");
        fs::write(&path, yaml).unwrap();

        // find_candidates should find it via the sanitised prefix
        let candidates = resolver.find_candidates("my.service/v2").unwrap();
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn resolve_with_covariates_empty_dir_returns_not_found() {
        use crate::service_contract::{CovariateCategory, CovariateDeclaration};

        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        let profile = CovariateProfile::builder().put("region", "US").build();
        let declarations = vec![CovariateDeclaration::new(
            "region",
            CovariateCategory::Infrastructure,
        )];

        let result = resolver.resolve_with_covariates("nonexistent", &profile, &declarations);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecResolveError::NotFound { .. }
        ));
    }

    #[test]
    fn find_candidates_nonexistent_dir_returns_not_found() {
        let resolver = SpecResolver::with_dir("/nonexistent/dir");
        let result = resolver.find_candidates("test");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SpecResolveError::NotFound { .. }
        ));
    }

    #[test]
    fn error_display_formats_correctly() {
        let err = SpecResolveError::NotFound {
            service_contract_id: "my-service".to_string(),
            path: PathBuf::from("/specs"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
        };
        assert!(err.to_string().contains("my-service"));
        assert!(err.to_string().contains("/specs"));

        let err = SpecResolveError::Selection {
            service_contract_id: "my-service".to_string(),
            source: SelectionError::NoCandidates {
                service_contract_id: "my-service".to_string(),
            },
        };
        assert!(err.to_string().contains("my-service"));
        assert!(err.to_string().contains("selection failed"));
    }

    #[test]
    fn error_source_is_accessible() {
        let err = SpecResolveError::NotFound {
            service_contract_id: "x".to_string(),
            path: PathBuf::from("/x"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
        };
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn resolve_file_rejects_missing_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-fp-test.yaml");

        // Write a valid YAML spec without a fingerprint
        let spec = sample_spec();
        let yaml = spec.to_yaml().unwrap();
        fs::write(&path, yaml).unwrap();

        let result = SpecResolver::resolve_file(&path);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no contentFingerprint"),
            "error should mention missing fingerprint"
        );
    }
}
