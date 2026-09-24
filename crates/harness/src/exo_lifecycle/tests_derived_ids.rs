// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

//! Regression matrix for the derived-identity width (issue #458, ADR 0077).
//!
//! The owner mints internal identities from a caller-supplied `execution_id`, and each derived
//! value is re-validated by the 128-byte internal predicate while the identity itself is admitted
//! up to the published 512-byte wire width. These cases prove a derived value's width is a
//! function of the prefix alone, so no admitted identity derives a value any layer refuses.
//! Source-only: nothing here touches a host, a provider or a game.

use super::*;
use crate::exo_lifecycle::derived_ids;
use crate::exo_lifecycle::types;

/// Every derived identity the owner can mint, for one admitted identity.
fn derived_for(execution_id: &str) -> Vec<String> {
    vec![
        derived_ids::derived_binding_id(execution_id),
        derived_ids::derived_prepared_id(execution_id),
        derived_ids::derived_provider_attempt_id(execution_id),
    ]
}

/// The declared ceiling is the widest prefix plus one token, so a prefix that grows past the
/// internal bound is caught here rather than by a stale magic number.
#[test]
fn the_declared_derived_ceiling_is_the_widest_prefix_plus_one_token() {
    let longest = derived_ids::DERIVED_EXECUTION_ID_PREFIXES
        .iter()
        .map(|prefix| prefix.len())
        .max();
    assert_eq!(longest, Some(19), "the widest derived prefix is 19 bytes");
    assert_eq!(
        MAX_DERIVED_ID_BYTES,
        derived_ids::longest_prefix(&derived_ids::DERIVED_EXECUTION_ID_PREFIXES)
            + derived_ids::IDENTITY_TOKEN_BYTES,
        "the ceiling is the widest prefix plus one identity token"
    );
    // Both sides are constants, so the relationship holds at compile time; the assertion stays
    // because it is the test that fails if either number moves.
    const {
        assert!(MAX_DERIVED_ID_BYTES < MAX_ID_BYTES);
    }
}

/// The point of the change: a derived value does not grow with the admitted identity, so the
/// published 512-byte wire width has no hidden ceiling above it.
#[test]
fn a_derived_identity_does_not_grow_with_the_admitted_identity() {
    let narrow = derived_for("e");
    let widest_admitted = derived_for(&"e".repeat(MAX_WIRE_ID_BYTES));
    assert_eq!(narrow.len(), widest_admitted.len());
    for (small, large) in narrow.iter().zip(widest_admitted.iter()) {
        assert_eq!(small.len(), large.len(), "{small} vs {large}");
        assert!(
            large.len() <= MAX_DERIVED_ID_BYTES,
            "{large} is {} bytes",
            large.len()
        );
    }
}

/// The whole admitted band derives a value every layer accepts, so no accepted input is left
/// unsettleable — the defect this change closes.
#[test]
fn every_admitted_identity_derives_an_identity_every_layer_accepts() {
    for width in [
        1usize,
        109,
        110,
        111,
        128,
        129,
        MAX_WIRE_ID_BYTES - 1,
        MAX_WIRE_ID_BYTES,
    ] {
        let value = "e".repeat(width);
        assert!(
            types::wire_id(&value),
            "width {width} is inside the published wire bound"
        );
        for derived in derived_for(&value) {
            assert!(
                types::id(&derived),
                "width {width} derives an identity the manifest refuses: {derived}"
            );
            assert!(
                crate::provider_session::valid_id(&derived),
                "width {width} derives an identity the broker refuses: {derived}"
            );
        }
    }
}

/// A derived identity is still injective over the admitted band: two distinct admitted
/// identities never collide on one derived identity, which is what keeps broker lookup exact.
#[test]
fn distinct_admitted_identities_never_share_a_derived_identity() {
    assert_ne!(
        derived_ids::derived_binding_id(&"e".repeat(512)),
        derived_ids::derived_binding_id(&"e".repeat(511))
    );
    assert_ne!(
        derived_ids::derived_binding_id("execution-a"),
        derived_ids::derived_binding_id("execution-b")
    );
    assert_eq!(
        derived_ids::derived_binding_id("execution-a"),
        derived_ids::derived_binding_id("execution-a")
    );
}

/// The three derivations are distinct namespaces, so a binding identity is never mistakable for a
/// prepared or provider-attempt identity of the same request.
#[test]
fn the_three_derivations_are_distinct_namespaces() {
    let derived = derived_for("execution-a");
    for (index, left) in derived.iter().enumerate() {
        for right in derived.iter().skip(index + 1) {
            assert_ne!(left, right);
        }
    }
}

/// The internal bound is unchanged: the refusal vocabulary stays the ordinary sanitized `Invalid`
/// and a wider identity is not admitted by this change.
#[test]
fn the_internal_bound_and_the_refusal_vocabulary_are_unchanged() {
    assert_eq!(MAX_ID_BYTES, 128);
    assert!(!types::id(&"e".repeat(129)));
    assert!(types::id(&"e".repeat(128)));
    let text = LifecycleError::Invalid.to_string();
    assert!(
        !text.contains("128") && !text.contains("512"),
        "the refusal must not disclose a bound: {text}"
    );
}
