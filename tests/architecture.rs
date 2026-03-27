//! Architecture enforcement tests.
//!
//! These tests verify that module dependency rules are respected.
//! The statistics module must have zero dependencies on other feotest modules.

use std::fs;
use std::path::Path;

/// Scans all Rust source files under `src/statistics/` and asserts that none
/// of them contain `use crate::` imports referring to modules outside of
/// `crate::statistics`.
#[test]
fn statistics_module_has_no_intra_crate_dependencies() {
    let statistics_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/statistics");
    let mut violations = Vec::new();

    visit_rs_files(&statistics_dir, &mut |path, line_number, line| {
        // Look for `use crate::` that doesn't continue with `statistics`
        let trimmed = line.trim();
        if trimmed.starts_with("use crate::") && !trimmed.starts_with("use crate::statistics") {
            violations.push(format!("{}:{}: {}", path.display(), line_number, trimmed));
        }
    });

    assert!(
        violations.is_empty(),
        "The statistics module must not depend on other feotest modules.\n\
         Found {} violation(s):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

/// Recursively visits all `.rs` files under `dir` and calls `check` for each line.
fn visit_rs_files(dir: &Path, check: &mut dyn FnMut(&Path, usize, &str)) {
    let entries = fs::read_dir(dir).expect("failed to read directory");
    for entry in entries {
        let entry = entry.expect("failed to read directory entry");
        let path = entry.path();
        if path.is_dir() {
            visit_rs_files(&path, check);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let contents = fs::read_to_string(&path).expect("failed to read file");
            for (i, line) in contents.lines().enumerate() {
                check(&path, i + 1, line);
            }
        }
    }
}
