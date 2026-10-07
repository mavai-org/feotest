//! Which of a run's assertions bind the test (Statistical Companion §12.6).
//!
//! Every assertion is enforced by default: every functional criterion and
//! every latency constraint, explicit or baseline-derived. The run-time
//! setting [`ENV_VAR`] makes whole dimensions advisory instead. An advisory
//! dimension is decided by the same rules on the same samples and reported
//! with its verdict, but it never fails the test; configuration refusals
//! apply whatever the setting.
//!
//! The setting is the operator's choice for one run — typically `latency`
//! on a development machine much slower than the environment a latency
//! requirement was written for — never a property of a contract or a test,
//! so there is no builder method for it.

use std::env;

use crate::statistics::rules::EnforcementMode;

/// Environment variable naming the dimensions a run makes advisory:
/// `functional`, `latency`, or both as `functional,latency`. Unset or
/// blank, every assertion is enforced.
pub const ENV_VAR: &str = "FEOTEST_ADVISORY";

/// The value that names the functional dimension.
const FUNCTIONAL: &str = "functional";

/// The value that names the latency dimension.
const LATENCY: &str = "latency";

/// The mode a run gives each dimension of the test verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
// mavai-ref: JVI-9GVFJ2S — do not remove (resolves in mavai-orchestrator)
pub struct AssertionEnforcement {
    functional: EnforcementMode,
    latency: EnforcementMode,
}

impl AssertionEnforcement {
    /// Every assertion enforced: the default.
    pub const ALL_ENFORCED: Self = Self {
        functional: EnforcementMode::Enforced,
        latency: EnforcementMode::Enforced,
    };

    /// Resolves the run's setting from [`ENV_VAR`].
    ///
    /// # Panics
    ///
    /// Panics when the variable names anything but `functional` and
    /// `latency` (see [`parse`](Self::parse)): a misconfigured setting is a
    /// defect in whichever harness set it, reported before any sample runs.
    #[must_use]
    pub fn from_environment() -> Self {
        env::var(ENV_VAR).map_or(Self::ALL_ENFORCED, |raw| Self::parse(&raw))
    }

    /// Parses a setting: a comma-separated list of `functional` and
    /// `latency`, case-insensitive, blanks around each item ignored. A blank
    /// setting leaves every assertion enforced.
    ///
    /// # Panics
    ///
    /// Panics on an unknown or empty item — a configuration error, never
    /// ignored.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let mut enforcement = Self::ALL_ENFORCED;
        if raw.trim().is_empty() {
            return enforcement;
        }
        for item in raw.split(',') {
            match item.trim().to_ascii_lowercase().as_str() {
                FUNCTIONAL => enforcement.functional = EnforcementMode::Advisory,
                LATENCY => enforcement.latency = EnforcementMode::Advisory,
                _ => panic!(
                    "{ENV_VAR} must name {FUNCTIONAL}, {LATENCY} or both as \
                     {FUNCTIONAL},{LATENCY}, got {raw:?}"
                ),
            }
        }
        enforcement
    }

    /// The functional dimension's mode.
    #[must_use]
    pub const fn functional(&self) -> EnforcementMode {
        self.functional
    }

    /// The latency dimension's mode.
    #[must_use]
    pub const fn latency(&self) -> EnforcementMode {
        self.latency
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADVISORY: EnforcementMode = EnforcementMode::Advisory;
    const ENFORCED: EnforcementMode = EnforcementMode::Enforced;

    fn modes(raw: &str) -> (EnforcementMode, EnforcementMode) {
        let enforcement = AssertionEnforcement::parse(raw);
        (enforcement.functional(), enforcement.latency())
    }

    #[test]
    fn every_assertion_is_enforced_by_default() {
        assert_eq!(
            AssertionEnforcement::default(),
            AssertionEnforcement::ALL_ENFORCED
        );
        assert_eq!(modes(""), (ENFORCED, ENFORCED));
        assert_eq!(modes("  "), (ENFORCED, ENFORCED));
    }

    #[test]
    fn each_value_makes_its_dimension_advisory() {
        assert_eq!(modes("functional"), (ADVISORY, ENFORCED));
        assert_eq!(modes("latency"), (ENFORCED, ADVISORY));
        assert_eq!(modes("functional,latency"), (ADVISORY, ADVISORY));
        assert_eq!(modes("latency,functional"), (ADVISORY, ADVISORY));
    }

    #[test]
    fn values_are_case_insensitive_and_trimmed() {
        assert_eq!(modes(" Latency "), (ENFORCED, ADVISORY));
        assert_eq!(modes("FUNCTIONAL , latency"), (ADVISORY, ADVISORY));
    }

    #[test]
    #[should_panic(expected = "FEOTEST_ADVISORY must name functional, latency or both")]
    fn an_unknown_value_is_a_configuration_error() {
        let _ = AssertionEnforcement::parse("strict");
    }

    #[test]
    #[should_panic(expected = "got \"latency,\"")]
    fn an_empty_item_is_a_configuration_error() {
        let _ = AssertionEnforcement::parse("latency,");
    }
}
