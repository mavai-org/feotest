//! Regression guard: internal tracking codes must never appear in source.
//!
//! The orchestrating project tracks features with short identifiers of the
//! form `<PREFIX><two digits>` (for example a two-letter prefix followed by
//! `09`). Those identifiers are meaningless to a reader of this crate and
//! leak a private taxonomy, so they are forbidden anywhere in this project's
//! Rust sources — production, tests, examples, and the proc-macro crate.
//!
//! This test walks every `.rs` file under the source, test, example, and
//! macro trees and fails if any forbidden identifier is present, listing each
//! offending `file:line`. It exempts its own file, which necessarily names the
//! prefixes it guards against.
//!
//! The opaque cross-reference anchors (`javai-ref: JVI-…`) use a prefix that
//! is not in the guarded set, so they do not trip this guard.

use std::fs;
use std::path::{Path, PathBuf};

/// Two-letter prefixes that, when followed by exactly two digits, form a
/// forbidden internal tracking code.
const FORBIDDEN_PREFIXES: &[&str] = &[
    "CT", "EX", "LT", "PT", "RC", "RP", "SC", "SN", "TH", "UC", "XM", "CR", "DG",
];

/// Directories (relative to the crate root) whose `.rs` files are scanned.
const SCAN_ROOTS: &[&str] = &["src", "tests", "examples", "feotest-macros/src"];

/// This guard's own file name, exempt because it lists the prefixes.
const SELF_FILE: &str = "code_isolation.rs";

#[test]
fn no_internal_tracking_codes_in_source() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders: Vec<String> = Vec::new();

    for root in SCAN_ROOTS {
        let dir = crate_root.join(root);
        if !dir.exists() {
            continue;
        }
        let mut files = Vec::new();
        collect_rs_files(&dir, &mut files);
        for file in files {
            if file
                .file_name()
                .is_some_and(|name| name == SELF_FILE)
            {
                continue;
            }
            scan_file(&file, crate_root, &mut offenders);
        }
    }

    assert!(
        offenders.is_empty(),
        "internal tracking codes found in source (use domain language instead):\n{}",
        offenders.join("\n")
    );
}

/// Recursively collects every `.rs` file under `dir`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Scans one file, appending a `file:line` record for each forbidden code.
fn scan_file(file: &Path, crate_root: &Path, offenders: &mut Vec<String>) {
    let contents =
        fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
    let relative = file.strip_prefix(crate_root).unwrap_or(file);
    for (index, line) in contents.lines().enumerate() {
        if line_contains_forbidden_code(line) {
            offenders.push(format!("{}:{}", relative.display(), index + 1));
        }
    }
}

/// Returns `true` if `line` contains a forbidden `<PREFIX><two digits>` token
/// with proper boundaries: the prefix is not preceded by an identifier
/// character and the two digits are not followed by another digit.
fn line_contains_forbidden_code(line: &str) -> bool {
    let bytes = line.as_bytes();
    for prefix in FORBIDDEN_PREFIXES {
        let prefix_bytes = prefix.as_bytes();
        let plen = prefix_bytes.len();
        let mut start = 0;
        while let Some(found) = find_from(bytes, prefix_bytes, start) {
            let code_end = found + plen + 2;
            // Need exactly two digits after the prefix.
            let two_digits = code_end <= bytes.len()
                && bytes[found + plen].is_ascii_digit()
                && bytes[found + plen + 1].is_ascii_digit();
            if !two_digits {
                start = found + 1;
                continue;
            }
            // Left boundary: prefix not part of a longer identifier.
            let left_ok = found == 0 || !is_ident_byte(bytes[found - 1]);
            // Right boundary: not a third digit (and not glued into a longer
            // identifier via another alphanumeric / underscore).
            let right_ok = code_end == bytes.len() || !is_ident_byte(bytes[code_end]);
            if left_ok && right_ok {
                return true;
            }
            start = found + 1;
        }
    }
    false
}

/// `true` for ASCII letters, digits, and underscore — the characters that can
/// be part of a Rust identifier (sufficient for boundary checks here).
const fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Finds `needle` in `haystack` starting at `from`, returning its index.
fn find_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|pos| from + pos)
}

#[cfg(test)]
mod tests {
    use super::line_contains_forbidden_code;

    #[test]
    fn flags_a_bare_code() {
        assert!(line_contains_forbidden_code("// PT09: failure inevitable"));
    }

    #[test]
    fn flags_code_at_line_edges() {
        assert!(line_contains_forbidden_code("RP07"));
        assert!(line_contains_forbidden_code("see SN02"));
    }

    #[test]
    fn ignores_code_inside_longer_identifier() {
        assert!(!line_contains_forbidden_code("let SPT09 = 1;"));
        assert!(!line_contains_forbidden_code("foo_PT09"));
    }

    #[test]
    fn ignores_prefix_glued_to_more_digits() {
        assert!(!line_contains_forbidden_code("PT091"));
        assert!(!line_contains_forbidden_code("the year PT2009"));
    }

    #[test]
    fn ignores_prefix_without_two_digits() {
        assert!(!line_contains_forbidden_code("PT9 only one digit"));
        assert!(!line_contains_forbidden_code("PTx"));
    }

    #[test]
    fn ignores_opaque_cross_reference_anchor() {
        assert!(!line_contains_forbidden_code(
            "// javai-ref: JVI-BQTS77W — do not remove"
        ));
    }
}
