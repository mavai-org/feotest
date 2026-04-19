//! Structured output of verdicts and diagnostics.
//!
//! The primary output format is `JUnit` XML, which is understood by CI systems
//! (GitHub Actions, GitLab CI, Jenkins) and by `cargo-nextest`.
//!
//! The verdict XML interchange format (RP07) serialises the full verdict
//! record for the report pipeline and cross-project tooling.
//!
//! Console rendering is available for interactive use and transparent
//! statistics mode.

pub mod console;
mod html;
mod junit;
pub mod transparent;
mod verdict_xml;

pub use console::ConsoleRenderer;
pub use html::HtmlReportWriter;
pub use junit::JunitXmlWriter;
pub use transparent::render as render_transparent_stats;
pub use transparent::render_verdict_line;
pub use verdict_xml::VerdictXmlWriter;
