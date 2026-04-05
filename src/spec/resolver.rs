//! Spec resolution: finding the right baseline for a use case.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::spec::BaselineSpec;
use crate::spec::namer::{CovariateProfile, baseline_filename, compute_footprint};

/// Resolves baseline specs from the filesystem.
///
/// Searches for specs by use case ID, checking an environment-override
/// directory first, then the configured default.
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

    /// Resolves a baseline spec for the given use case ID.
    ///
    /// Scans the spec directory for YAML files whose name starts with
    /// the sanitized use case ID followed by `-`. If multiple candidates
    /// exist, the first match is returned (future: covariate-aware
    /// selection).
    ///
    /// # Errors
    ///
    /// Returns an error if no matching file is found or parsing fails.
    pub fn resolve(&self, use_case_id: &str) -> Result<BaselineSpec, SpecResolveError> {
        let sanitized = use_case_id
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

        let entries = std::fs::read_dir(&self.spec_dir).map_err(|e| {
            SpecResolveError::NotFound {
                use_case_id: use_case_id.to_string(),
                path: self.spec_dir.clone(),
                source: e,
            }
        })?;

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&prefix) && name_str.ends_with(".yaml") {
                let path = entry.path();
                let content =
                    std::fs::read_to_string(&path).map_err(|e| SpecResolveError::NotFound {
                        use_case_id: use_case_id.to_string(),
                        path: path.clone(),
                        source: e,
                    })?;
                return BaselineSpec::from_yaml(&content)
                    .map_err(|e| SpecResolveError::ParseError { path, source: e });
            }
        }

        Err(SpecResolveError::NotFound {
            use_case_id: use_case_id.to_string(),
            path: self.spec_dir.join(format!("{prefix}*.yaml")),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no baseline file matching '{prefix}*.yaml'"),
            ),
        })
    }

    /// Reads a baseline spec directly from a file path.
    ///
    /// Use this when the exact file path is known (e.g., from a macro
    /// `spec` attribute) rather than resolving by use case ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the file is not found or cannot be parsed.
    pub fn resolve_file(path: impl AsRef<Path>) -> Result<BaselineSpec, SpecResolveError> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|e| SpecResolveError::NotFound {
            use_case_id: path.display().to_string(),
            path: path.to_path_buf(),
            source: e,
        })?;
        BaselineSpec::from_yaml(&content).map_err(|e| SpecResolveError::ParseError {
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Writes a baseline spec to the spec directory.
    ///
    /// The filename encodes the use case ID, footprint hash, and covariate
    /// value hashes. See [`crate::spec::namer`] for the filename format.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be written.
    pub fn write(
        &self,
        spec: &BaselineSpec,
        covariate_keys: &[&str],
        covariate_profile: &CovariateProfile,
    ) -> Result<PathBuf, std::io::Error> {
        std::fs::create_dir_all(&self.spec_dir)?;
        let footprint = compute_footprint(&spec.use_case_id, covariate_keys);
        let filename = baseline_filename(&spec.use_case_id, &footprint, covariate_profile);
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
        /// The use case ID that was looked up.
        use_case_id: String,
        /// The path that was checked.
        path: PathBuf,
        /// The underlying IO error.
        source: std::io::Error,
    },
    /// The spec file could not be parsed.
    ParseError {
        /// The path that was read.
        path: PathBuf,
        /// The underlying parse error.
        source: serde_yaml::Error,
    },
}

impl std::fmt::Display for SpecResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound {
                use_case_id, path, ..
            } => write!(
                f,
                "no spec found for use case '{use_case_id}' at {}",
                path.display()
            ),
            Self::ParseError { path, source } => {
                write!(f, "failed to parse spec at {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for SpecResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotFound { source, .. } => Some(source),
            Self::ParseError { source, .. } => Some(source),
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
            },
        )
    }

    #[test]
    fn write_and_resolve_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let resolver = SpecResolver::with_dir(dir.path());

        let spec = sample_spec();
        let profile = CovariateProfile::empty();
        let path = resolver.write(&spec, &[], &profile).unwrap();
        assert!(path.exists());
        assert!(path.file_name().unwrap().to_str().unwrap().starts_with("test-use-case-"));

        let resolved = resolver.resolve("test-use-case").unwrap();
        assert_eq!(resolved.use_case_id, "test-use-case");
        assert!((resolved.requirements.min_pass_rate - 0.85).abs() < 1e-10);
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
    fn resolve_malformed_yaml_returns_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let spec_path = dir.path().join("bad-spec.yaml");
        fs::write(&spec_path, "not: valid: yaml: [[[").unwrap();

        let resolver = SpecResolver::with_dir(dir.path());
        let result = resolver.resolve("bad-spec");
        assert!(result.is_err());
    }
}
