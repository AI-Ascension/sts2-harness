// SPDX-License-Identifier: MIT

//! Internal identities the owner mints by prefixing a caller-supplied `execution_id` (issue #458).
//!
//! Deriving an internal identity is deliberately *not* length-preserving. Each derived value is
//! re-validated by the same identity predicate that accepted the input, so an `execution_id` the
//! manifest accepts can derive a value the broker, the prepared-turn record or this crate refuses.
//! A request in that band would be admitted and then stay permanently unsettleable, so the owner
//! refuses the input up front and the accepted band and the derived band agree instead of
//! diverging by the prefix width.

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

/// Longest `execution_id` whose every derived internal identity still satisfies [`MAX_ID_BYTES`].
///
/// The bound is computed from the prefixes actually used, so shortening or replacing a prefix moves
/// the ceiling with it rather than leaving a stale magic number behind.
pub const MAX_DERIVABLE_EXECUTION_ID_BYTES: usize =
    MAX_ID_BYTES - longest_prefix(&DERIVED_EXECUTION_ID_PREFIXES);

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

/// Whether `execution_id` can derive every internal identity the owner mints from it.
///
/// Refusing here is what keeps an accepted request settleable: the caller sees the ordinary
/// sanitized refusal, and no internal identity width is disclosed.
#[must_use]
pub(crate) fn execution_id_is_derivable(execution_id: &str) -> bool {
    execution_id.len() <= MAX_DERIVABLE_EXECUTION_ID_BYTES
}
