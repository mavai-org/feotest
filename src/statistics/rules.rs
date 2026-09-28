//! The methodology's decision rules, configuration errors and test intent.
//!
//! Statistical Companion 1.5.0 names four verdict-producing procedures, each
//! with a versioned identifier that travels with every verdict it decides:
//!
//! - `regression/fisher` — empirical regression: the one-sided Fisher exact
//!   test as an integer cutoff on the test's success count (§3.4);
//! - `compliance/exact-binomial` — normative compliance: the exact one-sided
//!   binomial test as the smallest passing count (§3.6);
//! - `latency/precedence` — latency regression: the smallest baseline rank
//!   whose no-degradation breach probability is at most alpha (§12.4);
//! - `latency/compliance-exact-binomial` — an explicit latency requirement:
//!   the exact binomial test on the count of latencies within it (§12.3.4).
//!
//! Two configurations are refused before any sample runs, and a refusal
//! names every applicable code in one fixed order (§5.7.1).

use std::fmt;

use num_bigint::BigUint;
use serde::{Serialize, Serializer};

use crate::statistics::exact::exact_decimal;

/// The Statistical Companion methodology whose decision rules this crate
/// implements.
pub const METHODOLOGY_VERSION: &str = "1.5.0";

/// A versioned decision rule of the methodology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DecisionRule {
    /// Empirical regression: the one-sided Fisher exact test (§3.4).
    RegressionFisher,
    /// Normative compliance: the exact one-sided binomial test (§3.6).
    ComplianceExactBinomial,
    /// A baseline-derived latency threshold: the precedence rank (§12.4).
    LatencyPrecedence,
    /// An explicit latency requirement: the exact binomial test on the
    /// count of latencies within it (§12.3.4).
    LatencyComplianceExactBinomial,
}

impl DecisionRule {
    /// Every rule, in the order the methodology lists them.
    pub const ALL: [Self; 4] = [
        Self::RegressionFisher,
        Self::ComplianceExactBinomial,
        Self::LatencyPrecedence,
        Self::LatencyComplianceExactBinomial,
    ];

    /// The rule's identifier, as reports and interchange records state it.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::RegressionFisher => "regression/fisher",
            Self::ComplianceExactBinomial => "compliance/exact-binomial",
            Self::LatencyPrecedence => "latency/precedence",
            Self::LatencyComplianceExactBinomial => "latency/compliance-exact-binomial",
        }
    }

    /// The rule's version; every rule is at version 1 under methodology
    /// 1.5.0.
    #[must_use]
    pub const fn version(self) -> u32 {
        1
    }

    /// The error event this rule's alpha bounds (§1.4.6).
    #[must_use]
    pub const fn direction(self) -> Direction {
        match self {
            Self::ComplianceExactBinomial | Self::LatencyComplianceExactBinomial => {
                Direction::Compliance
            }
            Self::RegressionFisher | Self::LatencyPrecedence => Direction::Regression,
        }
    }
}

impl fmt::Display for DecisionRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl Serialize for DecisionRule {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// The error event a decision's alpha bounds, for the Type-I envelopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// A false claim of compliance: compliance criteria and enforced
    /// explicit latency requirements.
    Compliance,
    /// A false degradation signal: regression criteria and enforced
    /// baseline-derived latency constraints.
    Regression,
}

/// A configuration refused before any sample runs.
///
/// Declared in the fixed order in which a refusal reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConfigurationError {
    /// A test planned larger than the baseline run it consumes — the design
    /// policy that a baseline is at least as large as any test that consumes
    /// it, judged once on the two samplings, for pass-rate and latency
    /// criteria alike, whatever the intent.
    TestLargerThanBaseline,
    /// A normative design (a pass-rate requirement or an explicit latency
    /// requirement) too small for any outcome to demonstrate compliance,
    /// under verification intent.
    ComplianceInfeasible,
}

impl ConfigurationError {
    /// The error's code, as reports and interchange records state it.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::TestLargerThanBaseline => "TEST_LARGER_THAN_BASELINE",
            Self::ComplianceInfeasible => "COMPLIANCE_INFEASIBLE",
        }
    }
}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl Serialize for ConfigurationError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

/// Every applicable code once, in the fixed reporting order.
#[must_use]
pub fn ordered_configuration_errors(
    codes: impl IntoIterator<Item = ConfigurationError>,
) -> Vec<ConfigurationError> {
    let mut ordered: Vec<ConfigurationError> = codes.into_iter().collect();
    ordered.sort_unstable();
    ordered.dedup();
    ordered
}

/// The design rule `TEST_LARGER_THAN_BASELINE`, judged on the samplings.
///
/// Compares the test's planned sample size with the sample size of the
/// baseline run it consumes; latency success counts are never compared
/// (they are not known before the run and cannot exceed it).
#[must_use]
pub const fn check_test_size(
    baseline_samples: u32,
    planned_samples: u32,
) -> Option<ConfigurationError> {
    if planned_samples > baseline_samples {
        Some(ConfigurationError::TestLargerThanBaseline)
    } else {
        None
    }
}

/// The one-sided level `1 − confidence`, as the decimal it is written as.
///
/// `1 − 0.95` in binary floating point is `0.050000000000000044`; the level
/// a developer declared is `0.05`, and that is the value the exact-boundary
/// convention reads.
///
/// # Panics
///
/// Panics if `confidence` is not in the open interval `(0, 1)`.
#[must_use]
pub fn alpha_from_confidence(confidence: f64) -> f64 {
    assert!(
        confidence > 0.0 && confidence < 1.0,
        "confidence must be in (0, 1), got {confidence}"
    );
    let declared = exact_decimal(confidence);
    let (numerator, denominator) = declared.parts();
    let complement = denominator - numerator;
    decimal_fraction(&complement, denominator)
        .parse()
        .expect("a terminating decimal fraction always parses as a floating-point number")
}

/// `numerator / denominator` as a decimal string, for a denominator that
/// is a power of ten and a numerator below it.
fn decimal_fraction(numerator: &BigUint, denominator: &BigUint) -> String {
    let places = denominator.to_string().len() - 1;
    format!("0.{:0>places$}", numerator.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_carry_their_published_identifiers() {
        let ids: Vec<&str> = DecisionRule::ALL.iter().map(|r| r.id()).collect();
        assert_eq!(
            ids,
            [
                "regression/fisher",
                "compliance/exact-binomial",
                "latency/precedence",
                "latency/compliance-exact-binomial"
            ]
        );
        assert!(DecisionRule::ALL.iter().all(|r| r.version() == 1));
    }

    #[test]
    fn rules_enter_the_envelope_of_their_direction() {
        assert_eq!(
            DecisionRule::ComplianceExactBinomial.direction(),
            Direction::Compliance
        );
        assert_eq!(
            DecisionRule::LatencyComplianceExactBinomial.direction(),
            Direction::Compliance
        );
        assert_eq!(
            DecisionRule::RegressionFisher.direction(),
            Direction::Regression
        );
        assert_eq!(
            DecisionRule::LatencyPrecedence.direction(),
            Direction::Regression
        );
    }

    #[test]
    fn configuration_errors_are_reported_once_in_the_fixed_order() {
        let ordered = ordered_configuration_errors([
            ConfigurationError::ComplianceInfeasible,
            ConfigurationError::TestLargerThanBaseline,
            ConfigurationError::ComplianceInfeasible,
        ]);
        assert_eq!(
            ordered,
            [
                ConfigurationError::TestLargerThanBaseline,
                ConfigurationError::ComplianceInfeasible
            ]
        );
    }

    #[test]
    fn a_test_may_equal_but_not_exceed_its_baseline() {
        assert_eq!(check_test_size(100, 100), None);
        assert_eq!(
            check_test_size(100, 101),
            Some(ConfigurationError::TestLargerThanBaseline)
        );
    }

    #[test]
    fn alpha_is_the_declared_decimal_complement() {
        assert!((alpha_from_confidence(0.95) - 0.05).abs() == 0.0);
        assert!((alpha_from_confidence(0.99) - 0.01).abs() == 0.0);
        assert!((alpha_from_confidence(0.999) - 0.001).abs() == 0.0);
        assert!((alpha_from_confidence(0.996_875) - 0.003_125).abs() == 0.0);
    }
}
