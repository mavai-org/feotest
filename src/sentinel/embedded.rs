//! Embedded-baseline registry.
//!
//! Holds baselines baked into the sentinel binary at build time via
//! [`crate::sentinel::include_baselines!`] or an equivalent mechanism,
//! and resolved as the second step of the [`crate::sentinel::resolver`]
//! chain.

use crate::sentinel::resolver::{BaselineQuery, EmbeddedBaselineLookup};
use crate::spec::baseline::BaselineSpec;

/// One baseline entry, embedded into the binary at compile time.
///
/// Each entry carries the raw YAML string (not the pre-parsed
/// [`BaselineSpec`]) so fingerprint verification runs at resolution time
/// exactly as it does for baselines read from disk.
pub struct EmbeddedBaseline {
    /// Sentinel-registered name of the owning reliability specification.
    pub spec_name: &'static str,
    /// Method name of the paired probabilistic test.
    pub method_name: &'static str,
    /// Raw fingerprinted YAML content.
    pub yaml: &'static str,
}

inventory::collect!(EmbeddedBaseline);

/// Iterates every baseline baked into this binary.
pub fn registered_embedded_baselines() -> impl Iterator<Item = &'static EmbeddedBaseline> {
    inventory::iter::<EmbeddedBaseline>()
}

/// The production [`EmbeddedBaselineLookup`] — consults the global
/// inventory of [`EmbeddedBaseline`] entries. Zero-sized; constructible
/// anywhere without configuration.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultEmbeddedRegistry;

impl EmbeddedBaselineLookup for DefaultEmbeddedRegistry {
    fn lookup(&self, query: &BaselineQuery<'_>) -> Option<BaselineSpec> {
        // SN02 keys embedded defaults by (spec, method) only — covariate-
        // aware embedded lookup is follow-up work. The runtime accepts the
        // first matching entry whose YAML parses cleanly; malformed
        // embedded YAML is a build-time bug and panics the caller via the
        // `from_yaml` fingerprint check.
        for entry in registered_embedded_baselines() {
            if entry.spec_name == query.spec_name && entry.method_name == query.method_name {
                return BaselineSpec::from_yaml(entry.yaml).ok();
            }
        }
        None
    }
}
