//! Baseline specifications describing expected service behaviour.
//!
//! A specification captures the empirically measured pass rate of a service
//! under known conditions, along with metadata sufficient to reproduce and
//! contextualise the measurement.
//!
//! Specs are serialized to YAML for human readability and diff-friendliness.
//! They are the bridge between measure experiments (which produce them) and
//! probabilistic tests (which consume them).

pub mod baseline;
pub mod common;
pub mod explore;
pub mod projection;
mod resolver;

pub use baseline::BaselineSpec;
pub use resolver::SpecResolver;
