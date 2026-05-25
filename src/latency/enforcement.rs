//! Enforcement mode for baseline-derived latency thresholds.

use serde::{Serialize, Serializer};
use std::env;

/// Policy for baseline-derived latency thresholds.
///
/// Explicit thresholds declared on the builder are always enforced strictly.
/// Thresholds derived from a baseline spec follow this mode:
///
/// - `Advisory` — violations surface as warnings; the overall verdict is
///   unaffected.
/// - `Strict` — violations fail the overall verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
// javai-ref: JVI-9GVFJ2S — do not remove (resolves in javai-orchestrator)
pub enum LatencyEnforcementMode {
    /// Violations warn only. The default.
    #[default]
    Advisory,
    /// Violations fail the verdict.
    Strict,
}

impl Serialize for LatencyEnforcementMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::Advisory => "ADVISORY",
            Self::Strict => "STRICT",
        })
    }
}

/// Environment variable consulted when no builder setting is provided.
pub const ENV_VAR: &str = "FEOTEST_LATENCY_ENFORCE";

/// Resolves the mode from an optional builder setting and the environment.
///
/// Precedence: builder setting > `FEOTEST_LATENCY_ENFORCE` > `Advisory`.
/// Recognised env values (case-insensitive) for `Strict`: `1`, `true`,
/// `strict`. Anything else (including unset) yields `Advisory`.
#[must_use]
pub fn resolved_mode_from_env(
    builder_setting: Option<LatencyEnforcementMode>,
) -> LatencyEnforcementMode {
    if let Some(mode) = builder_setting {
        return mode;
    }
    env::var(ENV_VAR)
        .ok()
        .and_then(|v| parse_mode(&v))
        .unwrap_or(LatencyEnforcementMode::Advisory)
}

/// Parses an enforcement mode from a string value.
///
/// Recognised values (case-insensitive, whitespace-trimmed): `1`, `true`,
/// `strict`. Returns `None` for anything else.
fn parse_mode(value: &str) -> Option<LatencyEnforcementMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "strict" => Some(LatencyEnforcementMode::Strict),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_mode: pure logic, fully testable ---

    #[test]
    fn parse_1_is_strict() {
        assert_eq!(parse_mode("1"), Some(LatencyEnforcementMode::Strict));
    }

    #[test]
    fn parse_true_is_strict() {
        assert_eq!(parse_mode("true"), Some(LatencyEnforcementMode::Strict));
    }

    #[test]
    fn parse_strict_is_strict() {
        assert_eq!(parse_mode("strict"), Some(LatencyEnforcementMode::Strict));
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(parse_mode("STRICT"), Some(LatencyEnforcementMode::Strict));
        assert_eq!(parse_mode("True"), Some(LatencyEnforcementMode::Strict));
        assert_eq!(parse_mode("Strict"), Some(LatencyEnforcementMode::Strict));
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(parse_mode("  true  "), Some(LatencyEnforcementMode::Strict));
        assert_eq!(parse_mode("\t1\n"), Some(LatencyEnforcementMode::Strict));
    }

    #[test]
    fn parse_empty_is_none() {
        assert_eq!(parse_mode(""), None);
    }

    #[test]
    fn parse_invalid_is_none() {
        assert_eq!(parse_mode("maybe"), None);
        assert_eq!(parse_mode("0"), None);
        assert_eq!(parse_mode("false"), None);
        assert_eq!(parse_mode("advisory"), None);
    }

    // --- resolved_mode_from_env: builder-setting paths ---

    #[test]
    fn builder_advisory_takes_precedence() {
        assert_eq!(
            resolved_mode_from_env(Some(LatencyEnforcementMode::Advisory)),
            LatencyEnforcementMode::Advisory,
        );
    }

    #[test]
    fn builder_strict_takes_precedence() {
        assert_eq!(
            resolved_mode_from_env(Some(LatencyEnforcementMode::Strict)),
            LatencyEnforcementMode::Strict,
        );
    }

    #[test]
    fn no_builder_no_env_defaults_to_advisory() {
        // Assumes FEOTEST_LATENCY_ENFORCE is not set in the test environment.
        let result = resolved_mode_from_env(None);
        if env::var(ENV_VAR).is_err() {
            assert_eq!(result, LatencyEnforcementMode::Advisory);
        }
    }

    #[test]
    fn default_is_advisory() {
        assert_eq!(
            LatencyEnforcementMode::default(),
            LatencyEnforcementMode::Advisory,
        );
    }
}
