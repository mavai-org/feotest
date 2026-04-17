//! Structured output of verdicts and diagnostics.
//!
//! The primary output format is `JUnit` XML, which is understood by CI systems
//! (GitHub Actions, GitLab CI, Jenkins) and by `cargo-nextest`.
//!
//! Console rendering is available for interactive use and transparent
//! statistics mode.

pub mod console;
mod junit;
pub mod transparent;

pub use console::ConsoleRenderer;
pub use junit::JunitXmlWriter;
pub use transparent::render as render_transparent_stats;
pub use transparent::render_verdict_line;
