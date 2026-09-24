// SPDX-License-Identifier: MIT

//! Internal identities the owner mints by prefixing a caller-supplied `execution_id` (issue #458).
//!
//! Each derived value is re-validated by the 128-byte internal identity predicate, while the
//! request-level identity it is derived from is admitted up to the published 512-byte wire width.
//! Deriving by concatenation alone therefore diverged by the prefix width: an `execution_id` the
//! manifest accepts could derive a value the broker, the prepared-turn record or this crate
//! refuses, leaving an admitted request permanently unsettleable.
//!
//! A derived identity carries a digest of the identity rather than the identity itself, so its
//! width is a function of the prefix and the digest alone and is independent of how wide the
//! admitted identity is. Every admitted identity therefore derives a value the same predicate
//! accepts, and no width the wire admits has a hidden ceiling (ADR 0077).

use super::types::MAX_ID_BYTES;

/// Prefix the owner prepends to mint the broker binding identity from an `execution_id`.
pub(crate) const DERIVED_BINDING_PREFIX: &str = "lifecycle-binding-";
/// Prefix the owner prepends to mint the prepared-turn identity from an `execution_id`.
pub(crate) const DERIVED_PREPARED_PREFIX: &str = "lifecycle-prepared-";
/// Prefix the owner prepends to mint the provider-attempt identity from an `execution_id`.
pub(crate) const DERIVED_PROVIDER_ATTEMPT_PREFIX: &str = "provider-execution-";

/// Every prefix the owner prepends to an `execution_id`, so the ceiling below cannot drift from
/// the derivations that create the obligation.
pub(crate) const DERIVED_EXECUTION_ID_PREFIXES: [&str; 3] = [
    DERIVED_BINDING_PREFIX,
    DERIVED_PREPARED_PREFIX,
    DERIVED_PROVIDER_ATTEMPT_PREFIX,
];

/// Width of the lowercase hex SHA-256 that stands in for the identity inside a derived value.
pub(crate) const IDENTITY_TOKEN_BYTES: usize = 64;

/// Widest internal identity the owner mints: the widest prefix plus one identity token.
///
/// Computed from the prefixes actually used, so a prefix that grows past the internal identity
/// bound is caught by the test beside this constant rather than leaving a stale magic number
/// behind. It does not depend on the admitted identity width, which is the point.
pub const MAX_DERIVED_ID_BYTES: usize =
    longest_prefix(&DERIVED_EXECUTION_ID_PREFIXES) + IDENTITY_TOKEN_BYTES;

/// A derived identity must stay inside the internal identity bound; a prefix that grows past it
/// fails the build here rather than being discovered as a refusal at runtime.
const _: () = assert!(MAX_DERIVED_ID_BYTES <= MAX_ID_BYTES);

/// The widest derived prefix, in bytes.
pub(crate) const fn longest_prefix(prefixes: &[&str]) -> usize {
    let mut longest = 0;
    let mut index = 0;
    while index < prefixes.len() {
        let width = prefixes[index].len();
        if width > longest {
            longest = width;
        }
        index += 1;
    }
    longest
}

/// A bounded, deterministic token standing in for an admitted request identity.
///
/// The identity is digested rather than embedded, so a derived value's width does not grow with
/// the admitted width and the admitted width is never disclosed by a derived value.
#[must_use]
pub(crate) fn identity_token(execution_id: &str) -> String {
    crate::sha256_hex(execution_id)
}

/// The broker binding identity the owner mints for `execution_id`.
#[must_use]
pub(crate) fn derived_binding_id(execution_id: &str) -> String {
    format!("{DERIVED_BINDING_PREFIX}{}", identity_token(execution_id))
}

/// The prepared-turn identity the owner mints for `execution_id`.
#[must_use]
pub(crate) fn derived_prepared_id(execution_id: &str) -> String {
    format!("{DERIVED_PREPARED_PREFIX}{}", identity_token(execution_id))
}

/// The provider-attempt identity the owner mints for `execution_id`.
#[must_use]
pub(crate) fn derived_provider_attempt_id(execution_id: &str) -> String {
    format!(
        "{DERIVED_PROVIDER_ATTEMPT_PREFIX}{}",
        identity_token(execution_id)
    )
}
