//! Artefact key discipline: bounded identities for emitted YAML documents.
//!
//! The interchange artefacts this crate emits (exploration, optimization,
//! baseline specs) must parse in spec-strict YAML parsers. YAML caps
//! implicit mapping keys at 1,024 characters, so no emitted mapping key —
//! and no bounded identity string such as a failure entry's `condition` —
//! may grow with input or response content. Every such string stays within
//! [`MAX_KEY_CHARS`]; anything longer is truncated to a bounded prefix plus
//! a short content hash of the full original, so distinct over-long
//! identities remain distinct after truncation.

use sha2::{Digest, Sha256};

/// The bound on emitted mapping keys and identity strings, in characters.
///
/// Comfortably under YAML's 1,024-character implicit-key limit, and equal
/// to the bound the family's interchange schemas state for condition
/// identities and input excerpts.
pub const MAX_KEY_CHARS: usize = 256;

/// Hex digits of the content hash appended to a truncated identity.
const HASH_CHARS: usize = 8;

/// Marker inserted between the retained prefix and the content hash.
const TRUNCATION_MARKER: char = '…';

/// Bounds an identity string for artefact emission.
///
/// Returns the string unchanged when it is within [`MAX_KEY_CHARS`]
/// characters. Otherwise returns a bounded prefix, a truncation marker,
/// and an eight-hex-digit FNV-1a hash of the *full* original string —
/// exactly [`MAX_KEY_CHARS`] characters in total — so two identities that
/// share a prefix but differ later still emit distinct bounded forms.
#[must_use]
pub fn bounded_identity(raw: &str) -> String {
    let char_count = raw.chars().count();
    if char_count <= MAX_KEY_CHARS {
        return raw.to_owned();
    }
    let prefix_chars = MAX_KEY_CHARS - 1 - HASH_CHARS;
    let prefix: String = raw.chars().take(prefix_chars).collect();
    format!("{prefix}{TRUNCATION_MARKER}{:08x}", fnv1a_32(raw))
}

/// The bound on an exploration filename stem, in UTF-8 bytes.
///
/// Common file systems cap a file name at 255 bytes; 200 leaves room for
/// the `.yaml` extension. The same bounded stem is the exploration
/// document's `configuration` (the explore format's whole-stem bound,
/// amendment 2026-11-04).
pub const MAX_STEM_BYTES: usize = 200;

/// Hex digits of the SHA-256 appended to a bounded stem.
const STEM_HASH_CHARS: usize = 8;

/// Bounds an exploration filename stem.
///
/// Returns the stem unchanged when it is within [`MAX_STEM_BYTES`] UTF-8
/// bytes, so every stem that could be written before keeps its name.
/// Otherwise returns the longest prefix of whole characters within
/// `MAX_STEM_BYTES - 9` bytes, `-`, and the first eight hex digits of the
/// SHA-256 of the *whole* stem, so stems that differ only late remain
/// distinct. Unlike [`bounded_identity`], which bounds mapping keys and
/// condition identities in characters, this bound is in bytes, because a
/// file name is.
#[must_use]
pub fn bounded_stem(stem: &str) -> String {
    if stem.len() <= MAX_STEM_BYTES {
        return stem.to_owned();
    }
    let budget = MAX_STEM_BYTES - STEM_HASH_CHARS - 1;
    let prefix_end = stem
        .char_indices()
        .map(|(start, ch)| start + ch.len_utf8())
        .take_while(|&end| end <= budget)
        .last()
        .unwrap_or(0);
    let digest = format!("{:x}", Sha256::digest(stem.as_bytes()));
    format!("{}-{}", &stem[..prefix_end], &digest[..STEM_HASH_CHARS])
}

/// 32-bit FNV-1a hash over the string's UTF-8 bytes.
///
/// Chosen for stability: the truncated form of a given identity must be
/// identical across runs, builds, and releases (unlike `DefaultHasher`,
/// whose algorithm is unspecified).
const fn fnv1a_32(s: &str) -> u32 {
    const OFFSET_BASIS: u32 = 0x811c_9dc5;
    const PRIME: u32 = 0x0100_0193;
    let bytes = s.as_bytes();
    let mut hash = OFFSET_BASIS;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_within_the_byte_bound_pass_through_unchanged() {
        assert_eq!(bounded_stem("model-gpt-4o"), "model-gpt-4o");
        let exactly_at_bound = "s".repeat(MAX_STEM_BYTES);
        assert_eq!(bounded_stem(&exactly_at_bound), exactly_at_bound);
    }

    #[test]
    fn over_long_stems_count_bytes_and_never_split_a_character() {
        let stem = format!("prompt-x{}", "\u{e9}".repeat(150));
        let bounded = bounded_stem(&stem);
        assert!(bounded.len() <= MAX_STEM_BYTES);
        assert_eq!(bounded.len(), 199);
        assert!(bounded.starts_with(&format!("prompt-x{}-", "\u{e9}".repeat(91))));
    }

    #[test]
    fn stems_differing_only_late_stay_distinct() {
        let shared_prefix = "b".repeat(300);
        assert_ne!(
            bounded_stem(&format!("{shared_prefix}-one")),
            bounded_stem(&format!("{shared_prefix}-two"))
        );
    }

    #[test]
    fn short_identities_pass_through_unchanged() {
        assert_eq!(bounded_identity("well-formed"), "well-formed");
        let exactly_at_bound = "x".repeat(MAX_KEY_CHARS);
        assert_eq!(bounded_identity(&exactly_at_bound), exactly_at_bound);
    }

    #[test]
    fn over_long_identities_are_truncated_to_the_bound() {
        let long = "k".repeat(MAX_KEY_CHARS + 1);
        let bounded = bounded_identity(&long);
        assert_eq!(bounded.chars().count(), MAX_KEY_CHARS);
        assert!(bounded.starts_with(&"k".repeat(MAX_KEY_CHARS - 1 - HASH_CHARS)));
    }

    #[test]
    fn distinct_over_long_identities_stay_distinct() {
        let shared_prefix = "p".repeat(2_000);
        let a = format!("{shared_prefix}-first");
        let b = format!("{shared_prefix}-second");
        assert_ne!(bounded_identity(&a), bounded_identity(&b));
    }

    #[test]
    fn truncation_is_deterministic() {
        let long = "z".repeat(5_000);
        assert_eq!(bounded_identity(&long), bounded_identity(&long));
    }

    #[test]
    fn truncation_respects_multi_byte_character_boundaries() {
        let long = "ü".repeat(3_000);
        let bounded = bounded_identity(&long);
        assert_eq!(bounded.chars().count(), MAX_KEY_CHARS);
    }

    #[test]
    fn hash_matches_the_fnv1a_reference_values() {
        // Published FNV-1a test vectors.
        assert_eq!(fnv1a_32(""), 0x811c_9dc5);
        assert_eq!(fnv1a_32("a"), 0xe40c_292c);
        assert_eq!(fnv1a_32("foobar"), 0xbf9c_f968);
    }
}
