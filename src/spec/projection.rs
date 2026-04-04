//! Result projection: per-sample diagnostic detail for experiment output.
//!
//! Projections record every individual sample's input, response content,
//! postcondition results, and execution time. They are embedded inline in
//! exploration YAML files for diff-friendly diagnostic output.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use crate::model::TrialOutcome;

/// Per-sample diagnostic detail from an experiment trial.
#[derive(Debug, Clone)]
pub struct SampleProjection {
    sample_index: u32,
    input: Option<String>,
    postconditions: BTreeMap<String, PostconditionStatus>,
    execution_time_ms: u64,
    content: Option<String>,
    failure_detail: Option<String>,
}

impl SampleProjection {
    /// The zero-based sample index.
    #[must_use]
    pub const fn sample_index(&self) -> u32 {
        self.sample_index
    }

    /// The input used for this trial, if recorded.
    #[must_use]
    pub fn input(&self) -> Option<&str> {
        self.input.as_deref()
    }

    /// Postcondition check results.
    #[must_use]
    pub const fn postconditions(&self) -> &BTreeMap<String, PostconditionStatus> {
        &self.postconditions
    }

    /// Wall-clock duration of this trial in milliseconds.
    #[must_use]
    pub const fn execution_time_ms(&self) -> u64 {
        self.execution_time_ms
    }

    /// The full response content, if captured.
    #[must_use]
    pub fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }

    /// Error description if the trial failed exceptionally.
    #[must_use]
    pub fn failure_detail(&self) -> Option<&str> {
        self.failure_detail.as_deref()
    }

    /// Whether this sample succeeded (no failure detail).
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.failure_detail.is_none()
    }
}

/// Three-valued postcondition evaluation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostconditionStatus {
    /// The postcondition was satisfied.
    Passed,
    /// The postcondition was violated.
    Failed,
    /// The postcondition was not evaluated (fail-fast skipped it).
    Skipped,
}

impl fmt::Display for PostconditionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Passed => write!(f, "passed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

impl PostconditionStatus {
    /// Parses a status string.
    #[must_use]
    pub fn from_str_value(s: &str) -> Self {
        match s {
            "passed" => Self::Passed,
            "failed" => Self::Failed,
            _ => Self::Skipped,
        }
    }
}

/// Builds a [`SampleProjection`] from a [`TrialOutcome`] and execution context.
///
/// Extracts projection data from the outcome's metadata (content, postcondition
/// statuses) and the outcome's structural fields (elapsed time, violation).
#[must_use]
pub fn build_projection(
    sample_index: u32,
    input: &str,
    outcome: &TrialOutcome,
) -> SampleProjection {
    let execution_time_ms = u64::try_from(outcome.elapsed().as_millis()).unwrap_or(u64::MAX);

    let content = outcome.projection_content().map(str::to_owned);

    let failure_detail = outcome
        .violation()
        .map(|v| format!("{}: {}", v.check(), v.reason()));

    let mut postconditions = BTreeMap::new();
    for (name, status) in outcome.projection_postconditions() {
        postconditions.insert(name.to_owned(), PostconditionStatus::from_str_value(status));
    }

    let input_opt = if input.is_empty() {
        None
    } else {
        Some(input.to_owned())
    };

    SampleProjection {
        sample_index,
        input: input_opt,
        postconditions,
        execution_time_ms,
        content,
        failure_detail,
    }
}

// ---------------------------------------------------------------------------
// Diff anchor generation
// ---------------------------------------------------------------------------

/// Generates deterministic anchor comments for diff alignment.
///
/// Anchors are stable markers inserted between sample projections in YAML
/// output. Diff tools can align on these markers across experiment runs,
/// even when response content lengths vary.
///
/// The algorithm uses a seeded linear congruential generator matching
/// `java.util.Random(42)` so that anchor values are consistent across
/// the feotest and punit frameworks.
pub struct DiffAnchorGenerator;

impl DiffAnchorGenerator {
    /// The fixed seed for deterministic anchor generation.
    const SEED: u64 = 42;

    /// Generates the anchor line for a given sample index.
    ///
    /// Format: `# ────── anchor:XXXXXXXX ──────`
    #[must_use]
    pub fn anchor_line(sample_index: u32) -> String {
        let hash = Self::anchor_hash(sample_index);
        format!(
            "# \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500} anchor:{hash} \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
        )
    }

    /// Computes the 8-character hex hash for a sample index.
    ///
    /// Replicates `java.util.Random(42)`: seed with 42, call `nextLong()`
    /// N times to advance, then take the next value's lower 32 bits.
    #[must_use]
    fn anchor_hash(sample_index: u32) -> String {
        let mut state = java_random_seed(Self::SEED);

        // Advance N times
        for _ in 0..sample_index {
            let _ = java_random_next_long(&mut state);
        }

        // Take the next value
        let value = java_random_next_long(&mut state);
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let lower32 = value as u32;
        format!("{lower32:08x}")
    }
}

/// Java's `Random` seed scrambling: `(seed ^ 0x5DEECE66D) & ((1 << 48) - 1)`.
const fn java_random_seed(seed: u64) -> u64 {
    (seed ^ 0x5_DEEC_E66D) & ((1 << 48) - 1)
}

/// Java's `Random.next(bits)`: LCG step with `a = 0x5DEECE66D`, `c = 0xB`, mod 2^48.
const fn java_random_next(state: &mut u64, bits: u32) -> u32 {
    *state = (state.wrapping_mul(0x5_DEEC_E66D).wrapping_add(0xB)) & ((1 << 48) - 1);
    #[allow(clippy::cast_possible_truncation)]
    let result = (*state >> (48 - bits)) as u32;
    result
}

/// Java's `Random.nextLong()`: two calls to `next(32)`, composed into a 64-bit value.
fn java_random_next_long(state: &mut u64) -> i64 {
    let hi = i64::from(java_random_next(state, 32));
    let lo = i64::from(java_random_next(state, 32));
    (hi << 32) + lo
}

// ---------------------------------------------------------------------------
// YAML projection formatting
// ---------------------------------------------------------------------------

/// Formats result projections as YAML text matching punit's output format.
///
/// Each sample is preceded by a diff anchor comment and rendered as a
/// `sample[N]:` block with optional fields.
#[must_use]
pub fn format_projections(projections: &[SampleProjection]) -> String {
    if projections.is_empty() {
        return String::new();
    }

    let mut yaml = String::from("resultProjection:\n");

    for projection in projections {
        yaml.push_str(&DiffAnchorGenerator::anchor_line(projection.sample_index()));
        yaml.push('\n');
        format_single_projection(&mut yaml, projection);
    }

    yaml
}

fn format_single_projection(yaml: &mut String, p: &SampleProjection) {
    let idx = p.sample_index();
    let _ = writeln!(yaml, "  sample[{idx}]:");

    if let Some(input) = p.input() {
        let _ = writeln!(yaml, "    input: {}", yaml_escape(input));
    }

    if !p.postconditions().is_empty() {
        yaml.push_str("    postconditions:\n");
        for (name, status) in p.postconditions() {
            let _ = writeln!(yaml, "      {name}: {status}");
        }
    }

    let _ = writeln!(yaml, "    executionTimeMs: {}", p.execution_time_ms());

    if let Some(content) = p.content() {
        if content.is_empty() {
            yaml.push_str("    content: \"\"\n");
        } else {
            yaml.push_str("    content: |\n");
            for line in content.lines() {
                let _ = writeln!(yaml, "      {line}");
            }
        }
    }

    if let Some(detail) = p.failure_detail() {
        let _ = writeln!(yaml, "    failureDetail: {}", yaml_escape(detail));
    }
}

/// Escapes a string for safe YAML inline scalar output.
fn yaml_escape(s: &str) -> String {
    if s.contains(':')
        || s.contains('#')
        || s.contains('\'')
        || s.contains('"')
        || s.contains('\n')
        || s.starts_with(' ')
        || s.ends_with(' ')
    {
        format!("{s:?}")
    } else {
        s.to_owned()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn postcondition_status_display() {
        assert_eq!(PostconditionStatus::Passed.to_string(), "passed");
        assert_eq!(PostconditionStatus::Failed.to_string(), "failed");
        assert_eq!(PostconditionStatus::Skipped.to_string(), "skipped");
    }

    #[test]
    fn postcondition_status_from_str() {
        assert_eq!(
            PostconditionStatus::from_str_value("passed"),
            PostconditionStatus::Passed
        );
        assert_eq!(
            PostconditionStatus::from_str_value("failed"),
            PostconditionStatus::Failed
        );
        assert_eq!(
            PostconditionStatus::from_str_value("skipped"),
            PostconditionStatus::Skipped
        );
        assert_eq!(
            PostconditionStatus::from_str_value("unknown"),
            PostconditionStatus::Skipped
        );
    }

    #[test]
    fn build_projection_from_successful_trial() {
        let outcome = TrialOutcome::success(Duration::from_millis(42))
            .content("response text")
            .postcondition("Not empty", "passed")
            .postcondition("Valid JSON", "passed");

        let proj = build_projection(0, "Add apples", &outcome);

        assert_eq!(proj.sample_index(), 0);
        assert_eq!(proj.input(), Some("Add apples"));
        assert_eq!(proj.execution_time_ms(), 42);
        assert_eq!(proj.content(), Some("response text"));
        assert!(proj.failure_detail().is_none());
        assert!(proj.is_success());
        assert_eq!(proj.postconditions().len(), 2);
        assert_eq!(
            proj.postconditions()["Not empty"],
            PostconditionStatus::Passed
        );
    }

    #[test]
    fn build_projection_from_failed_trial() {
        use crate::model::ContractViolation;

        let outcome = TrialOutcome::failure(
            ContractViolation::new("parse", "invalid JSON"),
            Duration::from_millis(10),
        )
        .postcondition("Not empty", "passed")
        .postcondition("parse", "failed")
        .postcondition("Valid actions", "skipped");

        let proj = build_projection(3, "bad input", &outcome);

        assert_eq!(proj.sample_index(), 3);
        assert!(!proj.is_success());
        assert_eq!(proj.failure_detail(), Some("parse: invalid JSON"));
        assert_eq!(proj.postconditions().len(), 3);
        assert_eq!(proj.postconditions()["parse"], PostconditionStatus::Failed);
        assert_eq!(
            proj.postconditions()["Valid actions"],
            PostconditionStatus::Skipped
        );
    }

    #[test]
    fn build_projection_empty_input_becomes_none() {
        let outcome = TrialOutcome::success(Duration::from_millis(1));
        let proj = build_projection(0, "", &outcome);
        assert!(proj.input().is_none());
    }

    // --- Anchor tests ---

    #[test]
    fn anchor_is_deterministic() {
        let a = DiffAnchorGenerator::anchor_line(0);
        let b = DiffAnchorGenerator::anchor_line(0);
        assert_eq!(a, b);
    }

    #[test]
    fn different_indices_produce_different_anchors() {
        let a = DiffAnchorGenerator::anchor_line(0);
        let b = DiffAnchorGenerator::anchor_line(1);
        assert_ne!(a, b);
    }

    #[test]
    fn anchor_matches_punit_format() {
        let line = DiffAnchorGenerator::anchor_line(0);
        assert!(line.starts_with("# \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500} anchor:"));
        assert!(line.ends_with(" \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"));
        // Extract the hash portion — "# ────── anchor:" is 25 bytes, " ──────" is 19 bytes
        let prefix = "# \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500} anchor:";
        let suffix = " \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
        let hash = &line[prefix.len()..line.len() - suffix.len()];
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn anchor_values_match_punit_reference() {
        // These values are taken from punit's test output files
        assert_eq!(
            DiffAnchorGenerator::anchor_line(0),
            "# ────── anchor:0dfe8af7 ──────"
        );
        assert_eq!(
            DiffAnchorGenerator::anchor_line(1),
            "# ────── anchor:0c45c028 ──────"
        );
    }

    // --- YAML formatting tests ---

    #[test]
    fn format_projections_empty_returns_empty() {
        assert!(format_projections(&[]).is_empty());
    }

    #[test]
    fn format_projections_includes_anchor_and_sample() {
        let proj = build_projection(
            0,
            "Add milk",
            &TrialOutcome::success(Duration::from_millis(42))
                .content("ADD MILK")
                .postcondition("Not empty", "passed"),
        );

        let yaml = format_projections(&[proj]);
        assert!(yaml.starts_with("resultProjection:\n"));
        assert!(yaml.contains("anchor:0dfe8af7"));
        assert!(yaml.contains("sample[0]:"));
        assert!(yaml.contains("input: Add milk"));
        assert!(yaml.contains("Not empty: passed"));
        assert!(yaml.contains("executionTimeMs: 42"));
        assert!(yaml.contains("content: |\n"));
        assert!(yaml.contains("      ADD MILK"));
    }

    #[test]
    fn format_projections_multiple_samples() {
        let p0 = build_projection(
            0,
            "input-a",
            &TrialOutcome::success(Duration::from_millis(10)).content("A"),
        );
        let p1 = build_projection(
            1,
            "input-b",
            &TrialOutcome::success(Duration::from_millis(20)).content("B"),
        );

        let yaml = format_projections(&[p0, p1]);
        assert!(yaml.contains("sample[0]:"));
        assert!(yaml.contains("sample[1]:"));
        // Both anchors present
        assert!(yaml.contains("anchor:0dfe8af7"));
        assert!(yaml.contains("anchor:0c45c028"));
    }

    #[test]
    fn format_projection_with_failure_detail() {
        use crate::model::ContractViolation;

        let proj = build_projection(
            0,
            "bad input",
            &TrialOutcome::failure(
                ContractViolation::new("parse", "invalid JSON"),
                Duration::from_millis(5),
            )
            .postcondition("parse", "failed"),
        );

        let yaml = format_projections(&[proj]);
        assert!(yaml.contains("failureDetail:"));
        assert!(yaml.contains("parse: invalid JSON"));
    }
}
