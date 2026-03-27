//! Spec resolution: finding the right baseline for a use case.

use std::path::{Path, PathBuf};

use crate::spec::BaselineSpec;

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
    /// Looks for `{use_case_id}.yaml` in the spec directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the file is not found or cannot be parsed.
    pub fn resolve(&self, use_case_id: &str) -> Result<BaselineSpec, SpecResolveError> {
        let path = self.spec_dir.join(format!("{use_case_id}.yaml"));
        let content = std::fs::read_to_string(&path).map_err(|e| SpecResolveError::NotFound {
            use_case_id: use_case_id.to_string(),
            path: path.clone(),
            source: e,
        })?;

        BaselineSpec::from_yaml(&content)
            .map_err(|e| SpecResolveError::ParseError { path, source: e })
    }

    /// Writes a baseline spec to the spec directory.
    ///
    /// Creates the directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be written.
    pub fn write(&self, spec: &BaselineSpec) -> Result<PathBuf, std::io::Error> {
        std::fs::create_dir_all(&self.spec_dir)?;
        let path = self.spec_dir.join(format!("{}.yaml", spec.use_case_id));
        let yaml = spec.to_yaml().map_err(std::io::Error::other)?;
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
        let path = resolver.write(&spec).unwrap();
        assert!(path.exists());

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
