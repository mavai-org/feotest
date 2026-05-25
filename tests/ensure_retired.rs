//! Regression guard: the retired `ensure` authoring verb must not return.
//!
//! The criteria surface converged on `satisfies`; the old `ensure` /
//! `ensure_duration_below` verbs are gone. This guard scans the library source
//! and fails if either reappears as an identifier (a call or definition).
//! Prose such as "ensures that …" in doc comments is fine — the guard only
//! matches the verb used as code.

use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn ensure_authoring_verb_stays_retired() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);

    let mut offenders: Vec<String> = Vec::new();
    for file in files {
        let contents =
            fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for (index, line) in contents.lines().enumerate() {
            if line_uses_ensure_verb(line) {
                offenders.push(format!("{}:{}", file.display(), index + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the retired `ensure` verb reappeared — the converged verb is `satisfies`:\n{}",
        offenders.join("\n")
    );
}

/// `true` when the line uses `ensure` as code — a call or a definition — rather
/// than as prose ("ensures that …").
fn line_uses_ensure_verb(line: &str) -> bool {
    line.contains("ensure_duration_below")
        || line.contains(".ensure(")
        || line.contains("ensure(")
        || line.contains("fn ensure")
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::line_uses_ensure_verb;

    #[test]
    fn flags_ensure_calls_and_defs() {
        assert!(line_uses_ensure_verb(
            "        .ensure(\"check\", |r| r.ok())"
        ));
        assert!(line_uses_ensure_verb(
            "    fn ensure_duration_below(self) -> Self {"
        ));
        assert!(line_uses_ensure_verb("let x = ensure(cond);"));
    }

    #[test]
    fn ignores_prose() {
        assert!(!line_uses_ensure_verb(
            "/// Ensures that the response is parsed."
        ));
        assert!(!line_uses_ensure_verb(
            "// this ensures the invariant holds"
        ));
        assert!(!line_uses_ensure_verb(
            "/// The builder ensures uniqueness."
        ));
    }
}
