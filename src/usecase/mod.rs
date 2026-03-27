//! The unit of work under test.
//!
//! A use case represents a named service invocation together with its
//! associated configuration, contract, and operational controls. It is the
//! central organising concept that ties together what is being tested, how
//! success is defined, and under what conditions testing occurs.

use std::fmt;

/// A named, repeatable service invocation.
///
/// Implementations define the identity and metadata of a use case.
/// The actual service call logic lives in trial closures passed to
/// experiment and test builders, not in this trait.
pub trait UseCase: Send + Sync {
    /// Unique identifier for this use case.
    ///
    /// Used in spec filenames, reports, and CLI output.
    /// Convention: lowercase with dots as separators (e.g., `"shopping.product.search"`).
    fn id(&self) -> &str;

    /// Human-readable description.
    #[allow(clippy::unused_self, clippy::unnecessary_literal_bound)]
    fn description(&self) -> &str {
        ""
    }

    /// Number of warm-up invocations to discard before counting.
    ///
    /// Addresses cold-start non-stationarity (caches, connection pools, JIT).
    /// Warmup is additive: `samples=100` + `warmup=5` = 105 total invocations.
    fn warmup(&self) -> u32 {
        0
    }

    /// Covariate declarations for this use case.
    fn covariates(&self) -> Vec<CovariateDeclaration> {
        vec![]
    }
}

/// A use case that exposes configurable factors.
///
/// Experiments that need to manipulate configuration (explore, optimize)
/// require `T: UseCase + Configurable`. Use cases that do not expose
/// configurable factors simply do not implement this trait.
pub trait Configurable: UseCase {
    /// Returns the current value of a named factor.
    fn get_factor(&self, name: &str) -> Option<FactorValue>;

    /// Sets a named factor.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the factor name is unknown or the value is invalid.
    fn set_factor(&mut self, name: &str, value: FactorValue) -> Result<(), FactorError>;

    /// Lists the names of all configurable factors.
    fn factor_names(&self) -> Vec<&str>;
}

/// A value that can be assigned to a configurable factor.
#[derive(Debug, Clone, PartialEq)]
pub enum FactorValue {
    /// A string value.
    String(String),
    /// A floating-point value.
    Float(f64),
    /// An integer value.
    Int(i64),
    /// A boolean value.
    Bool(bool),
}

impl fmt::Display for FactorValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) => write!(f, "{s}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::Int(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
        }
    }
}

/// An error returned when a factor operation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorError {
    factor: String,
    reason: String,
}

impl FactorError {
    /// Creates a new factor error.
    #[must_use]
    pub fn new(factor: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            factor: factor.into(),
            reason: reason.into(),
        }
    }

    /// The name of the factor that caused the error.
    #[must_use]
    pub fn factor(&self) -> &str {
        &self.factor
    }

    /// Why the operation failed.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for FactorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "factor '{}': {}", self.factor, self.reason)
    }
}

impl std::error::Error for FactorError {}

/// The category of a covariate, affecting baseline matching semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovariateCategory {
    /// Hard gate — fails if no match during baseline selection.
    Configuration,
    /// Soft match with temporal-specific warning.
    Temporal,
    /// Soft match with infrastructure warning.
    Infrastructure,
    /// Soft match with operational warning.
    Operational,
    /// Soft match for external service dependencies.
    ExternalDependency,
    /// Soft match for data context.
    DataState,
}

/// A covariate declaration on a use case.
///
/// Covariates represent contextual factors that drive variance in system
/// behaviour. They are declared on use cases for baseline matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovariateDeclaration {
    key: String,
    category: CovariateCategory,
}

impl CovariateDeclaration {
    /// Creates a new covariate declaration.
    #[must_use]
    pub fn new(key: impl Into<String>, category: CovariateCategory) -> Self {
        Self {
            key: key.into(),
            category,
        }
    }

    /// The covariate key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The covariate category.
    #[must_use]
    pub const fn category(&self) -> CovariateCategory {
        self.category
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestUseCase;

    impl UseCase for TestUseCase {
        fn id(&self) -> &str {
            "test.use-case"
        }

        fn description(&self) -> &str {
            "A test use case"
        }

        fn warmup(&self) -> u32 {
            5
        }
    }

    #[test]
    fn use_case_provides_identity() {
        let uc = TestUseCase;
        assert_eq!(uc.id(), "test.use-case");
        assert_eq!(uc.description(), "A test use case");
        assert_eq!(uc.warmup(), 5);
    }

    #[test]
    fn default_warmup_is_zero() {
        struct Minimal;
        impl UseCase for Minimal {
            fn id(&self) -> &str {
                "minimal"
            }
        }
        assert_eq!(Minimal.warmup(), 0);
        assert_eq!(Minimal.description(), "");
        assert!(Minimal.covariates().is_empty());
    }

    #[test]
    fn factor_value_display() {
        assert_eq!(FactorValue::String("gpt-4o".into()).to_string(), "gpt-4o");
        assert_eq!(FactorValue::Float(0.7).to_string(), "0.7");
        assert_eq!(FactorValue::Int(42).to_string(), "42");
        assert_eq!(FactorValue::Bool(true).to_string(), "true");
    }

    #[test]
    fn factor_error_display() {
        let err = FactorError::new("temperature", "must be between 0 and 1");
        assert_eq!(
            err.to_string(),
            "factor 'temperature': must be between 0 and 1"
        );
    }

    struct ConfigurableUseCase {
        model: String,
    }

    impl UseCase for ConfigurableUseCase {
        fn id(&self) -> &str {
            "configurable"
        }
    }

    impl Configurable for ConfigurableUseCase {
        fn get_factor(&self, name: &str) -> Option<FactorValue> {
            match name {
                "model" => Some(FactorValue::String(self.model.clone())),
                _ => None,
            }
        }

        fn set_factor(&mut self, name: &str, value: FactorValue) -> Result<(), FactorError> {
            match name {
                "model" => {
                    if let FactorValue::String(s) = value {
                        self.model = s;
                        Ok(())
                    } else {
                        Err(FactorError::new("model", "expected string"))
                    }
                }
                _ => Err(FactorError::new(name, "unknown factor")),
            }
        }

        fn factor_names(&self) -> Vec<&str> {
            vec!["model"]
        }
    }

    #[test]
    fn configurable_get_and_set() {
        let mut uc = ConfigurableUseCase {
            model: "gpt-4o".into(),
        };
        assert_eq!(
            uc.get_factor("model"),
            Some(FactorValue::String("gpt-4o".into()))
        );

        uc.set_factor("model", FactorValue::String("claude-sonnet".into()))
            .unwrap();
        assert_eq!(
            uc.get_factor("model"),
            Some(FactorValue::String("claude-sonnet".into()))
        );
    }

    #[test]
    fn configurable_rejects_unknown_factor() {
        let mut uc = ConfigurableUseCase {
            model: "gpt-4o".into(),
        };
        let result = uc.set_factor("unknown", FactorValue::String("x".into()));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().factor(), "unknown");
    }

    #[test]
    fn covariate_declaration() {
        let cov = CovariateDeclaration::new("llm_model", CovariateCategory::ExternalDependency);
        assert_eq!(cov.key(), "llm_model");
        assert_eq!(cov.category(), CovariateCategory::ExternalDependency);
    }
}
