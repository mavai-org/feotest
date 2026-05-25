//! Baseline filename generation with covariate encoding.
//!
//! Baseline filenames encode the service contract identity and the environmental
//! conditions (covariates) under which the baseline was established. This
//! enables the spec resolver to select the most appropriate baseline for
//! the current test context.
//!
//! # Filename format
//!
//! ```text
//! {ServiceContractId}-{footprintHash}-{covHash1}-{covHash2}.yaml
//! ```
//!
//! - **ServiceContractId**: sanitized service contract name (unsafe characters replaced with `_`)
//! - **footprintHash**: 8-char SHA-256 of service contract ID + covariate *declarations*
//!   (names only, not values). Identifies *what* covariates exist.
//! - **covHash1..N**: 4-char SHA-256 per covariate of `key=value`. Identifies
//!   the specific *conditions*.
//!
//! When there are no covariates, the filename simplifies to
//! `{ServiceContractId}-{footprintHash}.yaml`.
//!
//! # Cross-framework compatibility
//!
//! This scheme follows the shared baseline file-naming scheme. The same service
//! contract with the same covariates produces the same filename structure across
//! frameworks (though hash values may differ due to implementation details).

use std::fmt::Write as _;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

/// Resolved covariate values captured at experiment time.
///
/// A profile holds the key-value pairs of all covariates as they were
/// at the moment the experiment ran. These values are hashed into the
/// baseline filename and written into the spec YAML.
#[derive(Debug, Clone, Default)]
pub struct CovariateProfile {
    entries: Vec<(String, String)>,
}

impl CovariateProfile {
    /// Creates an empty profile.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Creates a profile builder.
    #[must_use]
    pub fn builder() -> CovariateProfileBuilder {
        CovariateProfileBuilder::default()
    }

    /// Returns true if the profile has no covariates.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the covariate entries in declaration order.
    #[must_use]
    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    /// Resolves the current day of week as a canonical string.
    ///
    /// Returns one of: `WEEKDAY`, `WEEKEND`.
    #[must_use]
    pub fn resolve_day_of_week() -> String {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Days since epoch; 1970-01-01 was Thursday (day 4)
        let day = ((secs / 86400) + 4) % 7; // 0=Mon .. 6=Sun
        if day >= 5 { "WEEKEND" } else { "WEEKDAY" }.to_owned()
    }

    /// Resolves the current time of day as a canonical period string.
    ///
    /// Returns a 4-hour window like `"08:00/4h"`.
    #[must_use]
    pub fn resolve_time_of_day() -> String {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let hour = ((secs % 86400) / 3600) as u32;
        let window_start = (hour / 4) * 4;
        format!("{window_start:02}:00/4h")
    }

    /// Returns the value for a covariate key, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Computes a 4-character SHA-256 hash for each covariate key-value pair.
    ///
    /// Each hash is computed from `"{key}={value}"`. Different values for
    /// the same key produce different hashes.
    #[must_use]
    // javai-ref: JVI-07HPCY* — do not remove (resolves in javai-orchestrator)
    pub fn value_hashes(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|(k, v)| {
                let input = format!("{k}={v}");
                truncate_sha256(&input, 4)
            })
            .collect()
    }
}

/// Builder for [`CovariateProfile`].
#[derive(Debug, Default)]
pub struct CovariateProfileBuilder {
    entries: Vec<(String, String)>,
}

impl CovariateProfileBuilder {
    /// Adds a covariate value.
    #[must_use]
    pub fn put(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.entries.push((key.into(), value.into()));
        self
    }

    /// Builds the profile.
    #[must_use]
    pub fn build(self) -> CovariateProfile {
        CovariateProfile {
            entries: self.entries,
        }
    }
}

/// Computes the invocation footprint for a service contract.
///
/// The footprint uniquely identifies the combination of:
/// 1. Service contract identity
/// 2. Covariate declaration (names only, not values)
///
/// Two baselines with the same footprint are candidates for matching.
/// Covariate *values* then determine which candidate is selected.
#[must_use]
pub fn compute_footprint(service_contract_id: &str, covariate_keys: &[&str]) -> String {
    let mut input = String::new();
    let _ = writeln!(input, "usecase:{service_contract_id}");
    for key in covariate_keys {
        let _ = writeln!(input, "covariate:{key}");
    }
    truncate_sha256(&input, 8)
}

/// Generates the baseline filename with covariate encoding.
///
/// # Format
///
/// ```text
/// {ServiceContractId}-{footprintHash}-{covHash1}-{covHash2}.yaml
/// ```
///
/// With no covariates: `{ServiceContractId}-{footprintHash}.yaml`
#[must_use]
pub fn baseline_filename(
    service_contract_id: &str,
    footprint_hash: &str,
    covariate_profile: &CovariateProfile,
) -> String {
    let mut name = sanitize(service_contract_id);
    name.push('-');
    name.push_str(&truncate(footprint_hash, 4));

    for hash in covariate_profile.value_hashes() {
        name.push('-');
        name.push_str(&hash);
    }

    name.push_str(".yaml");
    name
}

/// Computes the first `len` hex characters of a SHA-256 hash of `input`.
fn truncate_sha256(input: &str, len: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    truncate(&hex, len)
}

/// Truncates a string to at most `len` characters.
fn truncate(s: &str, len: usize) -> String {
    s.chars().take(len).collect()
}

/// Replaces non-alphanumeric, non-underscore, non-hyphen characters with `_`.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_returns_value_for_existing_key() {
        let profile = CovariateProfile::builder()
            .put("region", "EU")
            .put("model", "gpt-4o")
            .build();
        assert_eq!(profile.get("region"), Some("EU"));
        assert_eq!(profile.get("model"), Some("gpt-4o"));
    }

    #[test]
    fn get_returns_none_for_missing_key() {
        let profile = CovariateProfile::builder().put("region", "EU").build();
        assert_eq!(profile.get("model"), None);
    }

    #[test]
    fn get_returns_none_for_empty_profile() {
        let profile = CovariateProfile::empty();
        assert_eq!(profile.get("region"), None);
    }

    #[test]
    fn empty_profile_produces_no_covariate_hashes() {
        let profile = CovariateProfile::empty();
        assert!(profile.value_hashes().is_empty());
    }

    #[test]
    fn value_hashes_are_four_chars() {
        let profile = CovariateProfile::builder()
            .put("region", "EU")
            .put("time_of_day", "MORNING")
            .build();

        let hashes = profile.value_hashes();
        assert_eq!(hashes.len(), 2);
        for hash in &hashes {
            assert_eq!(hash.len(), 4);
        }
    }

    #[test]
    fn different_values_produce_different_hashes() {
        let profile_a = CovariateProfile::builder().put("region", "EU").build();
        let profile_b = CovariateProfile::builder().put("region", "US").build();

        assert_ne!(profile_a.value_hashes(), profile_b.value_hashes());
    }

    #[test]
    fn footprint_is_eight_chars() {
        let fp = compute_footprint("ShoppingBasketServiceContract", &["region", "time_of_day"]);
        assert_eq!(fp.len(), 8);
    }

    #[test]
    fn footprint_stable_across_calls() {
        let fp1 = compute_footprint("ShoppingBasketServiceContract", &["region"]);
        let fp2 = compute_footprint("ShoppingBasketServiceContract", &["region"]);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn footprint_differs_with_different_covariates() {
        let fp1 = compute_footprint("ShoppingBasketServiceContract", &["region"]);
        let fp2 = compute_footprint("ShoppingBasketServiceContract", &["region", "time_of_day"]);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn filename_without_covariates() {
        let profile = CovariateProfile::empty();
        let fp = compute_footprint("ShoppingBasketServiceContract", &[]);
        let name = baseline_filename("ShoppingBasketServiceContract", &fp, &profile);

        assert!(name.starts_with("ShoppingBasketServiceContract-"));
        assert!(name.ends_with(".yaml"));
        // ServiceContractId + "-" + 4-char footprint + ".yaml"
        let parts: Vec<&str> = name.trim_end_matches(".yaml").split('-').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].len(), 4);
    }

    #[test]
    fn filename_with_covariates() {
        let profile = CovariateProfile::builder()
            .put("region", "EU")
            .put("time_of_day", "MORNING")
            .build();
        let fp = compute_footprint("ShoppingBasketServiceContract", &["region", "time_of_day"]);
        let name = baseline_filename("ShoppingBasketServiceContract", &fp, &profile);

        assert!(name.starts_with("ShoppingBasketServiceContract-"));
        assert!(name.ends_with(".yaml"));
        // ServiceContractId + "-" + 4-char footprint + "-" + 4-char + "-" + 4-char + ".yaml"
        let parts: Vec<&str> = name.trim_end_matches(".yaml").split('-').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[1].len(), 4); // footprint (truncated for filename)
        assert_eq!(parts[2].len(), 4); // cov hash 1
        assert_eq!(parts[3].len(), 4); // cov hash 2
    }

    #[test]
    fn full_footprint_is_eight_chars_filename_uses_four() {
        let fp = compute_footprint("ShoppingBasketServiceContract", &[]);
        assert_eq!(fp.len(), 8);

        let name = baseline_filename("ShoppingBasketServiceContract", &fp, &CovariateProfile::empty());
        let hash_in_name = name
            .trim_start_matches("ShoppingBasketServiceContract-")
            .trim_end_matches(".yaml");
        assert_eq!(hash_in_name.len(), 4);
        assert!(fp.starts_with(hash_in_name));
    }

    #[test]
    fn sanitize_replaces_unsafe_characters() {
        let name = baseline_filename("my.use/case", "abcd1234", &CovariateProfile::empty());
        assert!(name.starts_with("my_use_case-"));
    }
}
