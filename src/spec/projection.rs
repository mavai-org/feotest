//! Result projection: per-sample diagnostic detail for experiment output.
//!
//! Projections record every individual sample's input, response content,
//! postcondition results, and execution time. They are embedded inline in
//! exploration YAML files for diff-friendly diagnostic output.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::time::Duration;

use crate::criteria::CriterionSampleResult;

/// Per-sample diagnostic detail from an experiment trial.
#[derive(Debug, Clone)]
// javai-ref: JVI-G0R8DT$ — do not remove (resolves in javai-orchestrator)
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

/// Builds a [`SampleProjection`] from one sample's per-criterion results and
/// the measured invocation time.
///
/// Each criterion contributes a postcondition status (pass or fail); the first
/// failing criterion's violation becomes the failure detail.
#[must_use]
pub fn build_projection(
    sample_index: u32,
    input: &str,
    results: &[CriterionSampleResult],
    elapsed: Duration,
) -> SampleProjection {
    let execution_time_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);

    let failure_detail = results
        .iter()
        .find_map(CriterionSampleResult::reason)
        .map(|v| format!("{}: {}", v.check(), v.reason()));

    let mut postconditions = BTreeMap::new();
    for result in results {
        let status = if result.passed() {
            PostconditionStatus::Passed
        } else {
            PostconditionStatus::Failed
        };
        postconditions.insert(result.criterion().to_owned(), status);
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
        content: None,
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
/// The algorithm uses a seeded linear congruential generator (seed 42,
/// the multiplier/increment defined by the shared baseline scheme) so that
/// anchor values are consistent across frameworks.
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
        #[allow(
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation,
            reason = "truncates to low 32 bits for Java Random parity"
        )]
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
    #[allow(
        clippy::cast_possible_truncation,
        reason = "shift leaves at most bits (<= 32) significant bits"
    )]
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

/// Formats result projections as YAML text matching the shared output format.
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
            let key = yaml_escape(&crate::spec::keys::bounded_identity(name));
            let _ = writeln!(yaml, "      {key}: {status}");
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
///
/// Strings that read unambiguously as plain scalars pass through unchanged;
/// anything else is emitted as a YAML double-quoted scalar with spec-valid
/// escapes, so the whole emitted document stays parseable by spec-strict
/// YAML parsers regardless of runtime content.
fn yaml_escape(s: &str) -> String {
    if is_safe_plain_scalar(s) {
        s.to_owned()
    } else {
        yaml_double_quote(s)
    }
}

/// Whether a string can be emitted as a YAML plain scalar without changing
/// meaning: it must be non-empty, start with an alphanumeric character, not
/// carry surrounding whitespace, and contain none of the characters that
/// are structural or ambiguous inside a plain scalar in this writer's
/// flow-free context.
fn is_safe_plain_scalar(s: &str) -> bool {
    let starts_safely = s.chars().next().is_some_and(char::is_alphanumeric);
    starts_safely
        && !s.ends_with(' ')
        && !s.chars().any(|c| {
            matches!(c, ':' | '#' | '\'' | '"' | '\n' | '\t' | '\r' | '\\') || c.is_control()
        })
}

/// Renders a string as a YAML double-quoted scalar, escaping the characters
/// the double-quoted style requires (backslash, quote, and control
/// characters via YAML's own escape sequences).
fn yaml_double_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
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

    fn pass(criterion: &str) -> CriterionSampleResult {
        CriterionSampleResult::pass(criterion)
    }

    fn fail(criterion: &str, reason: &str) -> CriterionSampleResult {
        CriterionSampleResult::fail(
            criterion,
            crate::model::ContractViolation::new(criterion, reason),
        )
    }

    #[test]
    fn build_projection_from_successful_trial() {
        let results = [pass("Not empty"), pass("Valid JSON")];
        let proj = build_projection(0, "Add apples", &results, Duration::from_millis(42));

        assert_eq!(proj.sample_index(), 0);
        assert_eq!(proj.input(), Some("Add apples"));
        assert_eq!(proj.execution_time_ms(), 42);
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
        let results = [pass("Not empty"), fail("parse", "invalid JSON")];
        let proj = build_projection(3, "bad input", &results, Duration::from_millis(10));

        assert_eq!(proj.sample_index(), 3);
        assert!(!proj.is_success());
        assert_eq!(proj.failure_detail(), Some("parse: invalid JSON"));
        assert_eq!(proj.postconditions().len(), 2);
        assert_eq!(proj.postconditions()["parse"], PostconditionStatus::Failed);
        assert_eq!(
            proj.postconditions()["Not empty"],
            PostconditionStatus::Passed
        );
    }

    #[test]
    fn build_projection_empty_input_becomes_none() {
        let results = [pass("ok")];
        let proj = build_projection(0, "", &results, Duration::from_millis(1));
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
    fn anchor_matches_reference_format() {
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
    fn anchor_values_match_reference() {
        // These values are taken from the mavai conformance reference output
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
            &[pass("Not empty")],
            Duration::from_millis(42),
        );

        let yaml = format_projections(&[proj]);
        assert!(yaml.starts_with("resultProjection:\n"));
        assert!(yaml.contains("anchor:0dfe8af7"));
        assert!(yaml.contains("sample[0]:"));
        assert!(yaml.contains("input: Add milk"));
        assert!(yaml.contains("Not empty: passed"));
        assert!(yaml.contains("executionTimeMs: 42"));
    }

    #[test]
    fn format_projections_multiple_samples() {
        let p0 = build_projection(0, "input-a", &[pass("ok")], Duration::from_millis(10));
        let p1 = build_projection(1, "input-b", &[pass("ok")], Duration::from_millis(20));

        let yaml = format_projections(&[p0, p1]);
        assert!(yaml.contains("sample[0]:"));
        assert!(yaml.contains("sample[1]:"));
        // Both anchors present
        assert!(yaml.contains("anchor:0dfe8af7"));
        assert!(yaml.contains("anchor:0c45c028"));
    }

    #[test]
    fn escapes_values_with_yaml_escape_sequences_not_debug_formatting() {
        assert_eq!(yaml_escape("plain text"), "plain text");
        assert_eq!(yaml_escape("a: b"), "\"a: b\"");
        assert_eq!(yaml_escape("line\nbreak"), "\"line\\nbreak\"");
        // A control character must use a YAML escape, not Rust's \u{..} form.
        assert_eq!(yaml_escape("bell\u{7}"), "\"bell\\u0007\"");
        assert_eq!(yaml_escape("- looks structural"), "\"- looks structural\"");
        assert_eq!(yaml_escape(""), "\"\"");
    }

    #[test]
    fn over_long_postcondition_names_emit_bounded_keys() {
        let long_name = "n".repeat(2_000);
        let proj = build_projection(0, "in", &[pass(&long_name)], Duration::from_millis(1));
        let yaml = format_projections(&[proj]);

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let postconditions = &parsed["resultProjection"]["sample[0]"]["postconditions"];
        let keys: Vec<&str> = postconditions
            .as_mapping()
            .unwrap()
            .keys()
            .map(|k| k.as_str().unwrap())
            .collect();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].chars().count(), crate::spec::keys::MAX_KEY_CHARS);
    }

    #[test]
    fn format_projection_with_failure_detail() {
        let proj = build_projection(
            0,
            "bad input",
            &[fail("parse", "invalid JSON")],
            Duration::from_millis(5),
        );

        let yaml = format_projections(&[proj]);
        assert!(yaml.contains("failureDetail:"));
        assert!(yaml.contains("parse: invalid JSON"));
    }
}
