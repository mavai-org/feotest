//! Structured output of verdicts and diagnostics.
//!
//! The primary output format is `JUnit` XML, which is understood by CI systems
//! (GitHub Actions, GitLab CI, Jenkins) and by `cargo-nextest`.
//!
//! Console rendering is available for interactive use and transparent
//! statistics mode.

mod junit;

pub use junit::JunitXmlWriter;
