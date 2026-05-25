//! End-to-end integration tests that drive the `sentinel_demo` example
//! through the full sentinel CLI surface.
//!
//! Each test spawns `cargo run --example sentinel_demo -- <subcommand>`
//! in release-agnostic mode and asserts exit code + output shape. The
//! tests run sequentially because they share the workspace `target/`
//! directory for example builds — Cargo serialises concurrent builds
//! internally.

use std::path::Path;
use std::process::{Command, Output};

fn sentinel(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["run", "--quiet", "--example", "sentinel_demo", "--"]);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn sentinel_demo")
}

#[test]
fn list_enumerates_both_specs() {
    let out = sentinel(&["list"], &[]);
    assert!(out.status.success(), "list failed: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("sla_demo"),
        "stdout did not contain sla_demo: {stdout}"
    );
    assert!(
        stdout.contains("empirical_demo"),
        "stdout did not contain empirical_demo: {stdout}"
    );
}

#[test]
fn run_sla_only_passes_without_external_source() {
    let out = sentinel(&["run", "sla_demo"], &[]);
    assert!(
        out.status.success(),
        "expected success running sla_demo with no source; got {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("sla_demo.always_ok"),
        "verdict line not present: {stdout}"
    );
}

#[test]
fn run_empirical_panics_without_any_baseline_source() {
    let out = sentinel(&["run", "empirical_demo"], &[]);
    assert!(
        !out.status.success(),
        "expected failure running empirical_demo with no baseline; got success"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no embedded default baseline"),
        "panic message missing: {stderr}"
    );
}

#[test]
fn check_fails_when_empirical_has_no_baseline() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = sentinel(&["check", "--baselines", tmp.path().to_str().unwrap()], &[]);
    assert!(
        !out.status.success(),
        "check should fail with empty baselines dir"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("missing baseline"),
        "stderr should name missing baseline: {stderr}"
    );
}

#[test]
fn measure_then_run_round_trip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().to_str().unwrap();

    // 1. Measure mode writes a candidate baseline.
    let measure = sentinel(
        &[
            "measure",
            "--output",
            &format!("file://{dir}"),
            "empirical_demo",
        ],
        &[],
    );
    assert!(measure.status.success(), "measure failed: {measure:?}");
    let yaml_count = std::fs::read_dir(tmp.path())
        .expect("read tempdir")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("yaml"))
        .count();
    assert_eq!(
        yaml_count, 1,
        "expected one YAML baseline; found {yaml_count}"
    );

    // 2. Test mode reads that baseline via FEOTEST_BASELINE_SOURCE.
    let source = format!("file://{dir}");
    let run = sentinel(
        &["run", "empirical_demo"],
        &[("FEOTEST_BASELINE_SOURCE", source.as_str())],
    );
    assert!(
        run.status.success(),
        "run with external source failed: stdout={stdout}, stderr={stderr}",
        stdout = String::from_utf8_lossy(&run.stdout),
        stderr = String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("empirical_demo.matches_baseline"),
        "verdict line missing: {stdout}"
    );

    // 3. check should succeed when pointed at the same directory.
    let check = sentinel(&["check", "--baselines", dir], &[]);
    assert!(check.status.success(), "check failed: {check:?}");

    // tempdir exists — avoid compiler warning about unused `tmp`.
    let _ = Path::new(dir).exists();
}
