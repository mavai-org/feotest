//! The unit of work under test.
//!
//! A service contract represents a named service invocation together with its
//! associated configuration, contract, and operational controls. It is the
//! central organising concept that ties together what is being tested, how
//! success is defined, and under what conditions testing occurs.

use std::fmt;

use crate::spec::namer::CovariateProfile;

/// A named, repeatable service invocation.
///
/// Implementations define the identity and metadata of a service contract.
/// The actual service call logic lives in trial closures passed to
/// experiment and test builders, not in this trait.
pub trait ServiceContract: Send + Sync {
    /// Unique identifier for this service contract.
    ///
    /// Used in spec filenames, reports, and CLI output.
    /// Convention: lowercase with dots as separators (e.g., `"shopping.product.search"`).
    fn id(&self) -> &str;

    /// Human-readable description.
    #[allow(
        clippy::unused_self,
        clippy::unnecessary_literal_bound,
        reason = "default trait impl — signature must match trait"
    )]
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

    /// Covariate declarations for this service contract.
    fn covariates(&self) -> Vec<CovariateDeclaration> {
        vec![]
    }

    /// Resolves covariate values at the current point in time.
    ///
    /// Returns a profile with resolved values for all declared covariates.
    /// The default implementation returns an empty profile. Service contracts that
    /// declare covariates should override this to provide resolved values.
    fn resolve_covariates(&self) -> CovariateProfile {
        CovariateProfile::empty()
    }
}

/// A service contract that exposes configurable factors.
///
/// Experiments that need to manipulate configuration (explore, optimize)
/// require `T: ServiceContract + Configurable`. Service contracts that do not expose
/// configurable factors simply do not implement this trait.
pub trait Configurable: ServiceContract {
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

impl CovariateCategory {
    /// Whether this category acts as a hard gate during baseline selection.
    ///
    /// Configuration covariates must match exactly; all others are scored
    /// as soft matches with warnings on mismatch.
    #[must_use]
    pub const fn is_hard_gate(self) -> bool {
        matches!(self, Self::Configuration)
    }
}

impl fmt::Display for CovariateCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration => write!(f, "Configuration"),
            Self::Temporal => write!(f, "Temporal"),
            Self::Infrastructure => write!(f, "Infrastructure"),
            Self::Operational => write!(f, "Operational"),
            Self::ExternalDependency => write!(f, "ExternalDependency"),
            Self::DataState => write!(f, "DataState"),
        }
    }
}

/// A covariate declaration on a service contract.
///
/// Covariates represent contextual factors that drive variance in system
/// behaviour. They are declared on service contracts for baseline matching.
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

    /// Built-in: sensitivity to day of week.
    #[must_use]
    pub fn day_of_week() -> Self {
        Self::new("day-of-week", CovariateCategory::Temporal)
    }

    /// Built-in: sensitivity to time of day.
    #[must_use]
    pub fn time_of_day() -> Self {
        Self::new("time-of-day", CovariateCategory::Temporal)
    }

    /// Built-in: sensitivity to deployment region.
    #[must_use]
    pub fn region() -> Self {
        Self::new("region", CovariateCategory::Infrastructure)
    }

    /// Built-in: sensitivity to timezone.
    #[must_use]
    pub fn timezone() -> Self {
        Self::new("timezone", CovariateCategory::Infrastructure)
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

impl fmt::Display for CovariateDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.key, self.category)
    }
}

/// Covariate context for baseline selection.
///
/// Bundles the declarations (what covariates exist) with the resolved
/// profile (what values they have at the current point in time).
#[derive(Debug, Clone)]
pub struct CovariateContext {
    declarations: Vec<CovariateDeclaration>,
    profile: CovariateProfile,
}

impl CovariateContext {
    /// Creates a covariate context from a service contract.
    ///
    /// Extracts declarations and resolves the current profile. Returns
    /// `None` if the service contract declares no covariates.
    #[must_use]
    pub fn from_service_contract(service_contract: &dyn ServiceContract) -> Option<Self> {
        let declarations = service_contract.covariates();
        if declarations.is_empty() {
            return None;
        }
        let profile = service_contract.resolve_covariates();
        Some(Self {
            declarations,
            profile,
        })
    }

    /// The covariate declarations.
    #[must_use]
    pub fn declarations(&self) -> &[CovariateDeclaration] {
        &self.declarations
    }

    /// The resolved covariate profile.
    #[must_use]
    pub const fn profile(&self) -> &CovariateProfile {
        &self.profile
    }
}

/// Validates that covariate declarations have unique keys.
///
/// # Panics
///
/// Panics if two or more declarations share the same key.
pub fn validate_covariates(covariates: &[CovariateDeclaration]) {
    let mut seen = std::collections::HashSet::new();
    for cov in covariates {
        assert!(
            seen.insert(cov.key()),
            "duplicate covariate key '{}': each covariate must have a unique name within a service contract",
            cov.key()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestServiceContract;

    impl ServiceContract for TestServiceContract {
        fn id(&self) -> &str {
            "test.use-case"
        }

        fn description(&self) -> &str {
            "A test service contract"
        }

        fn warmup(&self) -> u32 {
            5
        }
    }

    #[test]
    fn service_contract_provides_identity() {
        let uc = TestServiceContract;
        assert_eq!(uc.id(), "test.use-case");
        assert_eq!(uc.description(), "A test service contract");
        assert_eq!(uc.warmup(), 5);
    }

    #[test]
    fn default_warmup_is_zero() {
        struct Minimal;
        impl ServiceContract for Minimal {
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

    struct ConfigurableServiceContract {
        model: String,
    }

    impl ServiceContract for ConfigurableServiceContract {
        fn id(&self) -> &str {
            "configurable"
        }
    }

    impl Configurable for ConfigurableServiceContract {
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
        let mut uc = ConfigurableServiceContract {
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
        let mut uc = ConfigurableServiceContract {
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

    #[test]
    fn built_in_covariate_helpers() {
        let dow = CovariateDeclaration::day_of_week();
        assert_eq!(dow.key(), "day-of-week");
        assert_eq!(dow.category(), CovariateCategory::Temporal);

        let tod = CovariateDeclaration::time_of_day();
        assert_eq!(tod.key(), "time-of-day");
        assert_eq!(tod.category(), CovariateCategory::Temporal);

        let reg = CovariateDeclaration::region();
        assert_eq!(reg.key(), "region");
        assert_eq!(reg.category(), CovariateCategory::Infrastructure);

        let tz = CovariateDeclaration::timezone();
        assert_eq!(tz.key(), "timezone");
        assert_eq!(tz.category(), CovariateCategory::Infrastructure);
    }

    #[test]
    fn covariate_category_display() {
        assert_eq!(
            CovariateCategory::Configuration.to_string(),
            "Configuration"
        );
        assert_eq!(CovariateCategory::Temporal.to_string(), "Temporal");
        assert_eq!(
            CovariateCategory::Infrastructure.to_string(),
            "Infrastructure"
        );
        assert_eq!(CovariateCategory::Operational.to_string(), "Operational");
        assert_eq!(
            CovariateCategory::ExternalDependency.to_string(),
            "ExternalDependency"
        );
        assert_eq!(CovariateCategory::DataState.to_string(), "DataState");
    }

    #[test]
    fn covariate_declaration_display() {
        let cov = CovariateDeclaration::new("llm_model", CovariateCategory::ExternalDependency);
        assert_eq!(cov.to_string(), "llm_model (ExternalDependency)");
    }

    #[test]
    fn validate_covariates_accepts_unique_keys() {
        let covs = vec![
            CovariateDeclaration::day_of_week(),
            CovariateDeclaration::region(),
            CovariateDeclaration::new("llm_model", CovariateCategory::ExternalDependency),
        ];
        validate_covariates(&covs); // should not panic
    }

    #[test]
    fn validate_covariates_accepts_empty() {
        validate_covariates(&[]); // should not panic
    }

    #[test]
    #[should_panic(expected = "duplicate covariate key 'region'")]
    fn validate_covariates_rejects_duplicates() {
        let covs = vec![
            CovariateDeclaration::region(),
            CovariateDeclaration::new("region", CovariateCategory::Operational),
        ];
        validate_covariates(&covs);
    }

    #[test]
    fn service_contract_with_covariates() {
        struct WithCovariates;
        impl ServiceContract for WithCovariates {
            fn id(&self) -> &str {
                "with-covariates"
            }
            fn covariates(&self) -> Vec<CovariateDeclaration> {
                vec![
                    CovariateDeclaration::day_of_week(),
                    CovariateDeclaration::time_of_day(),
                    CovariateDeclaration::new("llm_model", CovariateCategory::ExternalDependency),
                ]
            }
        }

        let uc = WithCovariates;
        let covs = uc.covariates();
        assert_eq!(covs.len(), 3);
        assert_eq!(covs[0].key(), "day-of-week");
        assert_eq!(covs[1].key(), "time-of-day");
        assert_eq!(covs[2].key(), "llm_model");
        validate_covariates(&covs);
    }

    #[test]
    fn configuration_is_hard_gate() {
        assert!(CovariateCategory::Configuration.is_hard_gate());
    }

    #[test]
    fn non_configuration_categories_are_not_hard_gates() {
        assert!(!CovariateCategory::Temporal.is_hard_gate());
        assert!(!CovariateCategory::Infrastructure.is_hard_gate());
        assert!(!CovariateCategory::Operational.is_hard_gate());
        assert!(!CovariateCategory::ExternalDependency.is_hard_gate());
        assert!(!CovariateCategory::DataState.is_hard_gate());
    }

    #[test]
    fn covariate_context_from_service_contract_with_covariates() {
        struct WithCovs;
        impl ServiceContract for WithCovs {
            fn id(&self) -> &str {
                "ctx-test"
            }
            fn covariates(&self) -> Vec<CovariateDeclaration> {
                vec![CovariateDeclaration::region()]
            }
            fn resolve_covariates(&self) -> CovariateProfile {
                CovariateProfile::builder().put("region", "EU").build()
            }
        }

        let ctx = CovariateContext::from_service_contract(&WithCovs).unwrap();
        assert_eq!(ctx.declarations().len(), 1);
        assert_eq!(ctx.profile().get("region"), Some("EU"));
    }

    #[test]
    fn covariate_context_none_for_no_covariates() {
        assert!(CovariateContext::from_service_contract(&TestServiceContract).is_none());
    }
}
