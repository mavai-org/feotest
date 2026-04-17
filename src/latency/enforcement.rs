//! Enforcement mode for baseline-derived latency thresholds.

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
pub enum LatencyEnforcementMode {
    /// Violations warn only. The default.
    #[default]
    Advisory,
    /// Violations fail the verdict.
    Strict,
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
        .map(|v| v.trim().to_ascii_lowercase())
        .and_then(|v| match v.as_str() {
            "1" | "true" | "strict" => Some(LatencyEnforcementMode::Strict),
            _ => None,
        })
        .unwrap_or(LatencyEnforcementMode::Advisory)
}
