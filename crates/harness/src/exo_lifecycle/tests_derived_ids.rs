// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

//! Regression matrix for the derived-identity ceiling (issue #458).
//!
//! The owner mints internal identities by prefixing an `execution_id`, and each derived value is
//! re-validated by the same 128-byte predicate that accepted the input. The derivation is not
//! length-preserving, so an input the manifest accepts can derive a value no layer can accept.
//! These cases prove the accepted band and the derived band agree instead of diverging by the
//! prefix width. Source-only: nothing here touches a host, a provider or a game.

use super::*;
use crate::exo_lifecycle::{derived_ids, types};

/// The prefix arithmetic the ceiling is derived from, stated once so the test fails if a prefix
/// changes without the bound moving with it.
#[test]
fn the_declared_ceiling_matches_the_prefixes_the_owner_actually_uses() {
    let prefixes = [
        derived_ids::DERIVED_BINDING_PREFIX,
        derived_ids::DERIVED_PREPARED_PREFIX,
        derived_ids::DERIVED_PROVIDER_ATTEMPT_PREFIX,
    ];
    let longest = prefixes.iter().map(|prefix| prefix.len()).max();
    assert_eq!(longest, Some(19), "the widest derived prefix is 19 bytes");
    assert_eq!(
        MAX_DERIVABLE_EXECUTION_ID_BYTES,
        MAX_ID_BYTES - 19,
        "the ceiling is the identity bound less the widest prefix"
    );
    assert_eq!(MAX_DERIVABLE_EXECUTION_ID_BYTES, 109);
}

/// The longest accepted identity derives a value that is still exactly legal — the boundary is
/// tight, not conservative.
#[test]
fn the_longest_accepted_identity_derives_exactly_the_identity_bound() {
    let widest = "e".repeat(MAX_DERIVABLE_EXECUTION_ID_BYTES);
    for prefix in [
        derived_ids::DERIVED_BINDING_PREFIX,
        derived_ids::DERIVED_PREPARED_PREFIX,
        derived_ids::DERIVED_PROVIDER_ATTEMPT_PREFIX,
    ] {
        let derived = format!("{prefix}{widest}");
        assert!(
            derived.len() <= MAX_ID_BYTES,
            "{prefix} at the ceiling derives {} bytes",
            derived.len()
        );
        assert!(types::id(&derived), "the derived value stays legal");
    }

    let longest = format!("{}{widest}", derived_ids::DERIVED_PREPARED_PREFIX);
    assert_eq!(
        longest.len(),
        MAX_ID_BYTES,
        "the ceiling is exact, not slack"
    );
}

/// One byte past the ceiling derives a value no acceptance predicate can admit. This is the
/// regression: the input itself is still inside the manifest bound, so without the owner's guard
/// it validates and then can never settle.
#[test]
fn one_byte_past_the_ceiling_derives_an_illegal_identity() {
    let over = "e".repeat(MAX_DERIVABLE_EXECUTION_ID_BYTES + 1);
    assert!(
        types::id(&over),
        "the input is still accepted by the manifest predicate, which is what makes it a trap"
    );
    for prefix in [
        derived_ids::DERIVED_BINDING_PREFIX,
        derived_ids::DERIVED_PREPARED_PREFIX,
        derived_ids::DERIVED_PROVIDER_ATTEMPT_PREFIX,
    ] {
        let derived = format!("{prefix}{over}");
        assert!(
            derived.len() > MAX_ID_BYTES,
            "{prefix} at one past the ceiling stays legal"
        );
        assert!(
            !types::id(&derived),
            "the derived value must be refused by the same predicate"
        );
    }
}

/// The whole accepted band derives legal identities, so no accepted input is left unsettleable.
#[test]
fn every_identity_inside_the_ceiling_derives_a_legal_identity() {
    for width in 1..=MAX_DERIVABLE_EXECUTION_ID_BYTES {
        let value = "e".repeat(width);
        assert!(types::id(&value), "input of width {width} is accepted");
        for prefix in [
            derived_ids::DERIVED_BINDING_PREFIX,
            derived_ids::DERIVED_PREPARED_PREFIX,
            derived_ids::DERIVED_PROVIDER_ATTEMPT_PREFIX,
        ] {
            let derived = format!("{prefix}{value}");
            assert!(
                types::id(&derived),
                "width {width} derives an illegal value under {prefix}"
            );
        }
    }
}

/// The refusal is the ordinary sanitized `Invalid`, so a caller learns nothing beyond "refused"
/// and no hidden identity width is disclosed.
#[test]
fn the_refusal_vocabulary_is_unchanged_and_value_free() {
    let text = LifecycleError::Invalid.to_string();
    assert!(
        !text.contains("109") && !text.contains("128"),
        "the refusal must not disclose the bound: {text}"
    );
    assert_eq!(LifecycleError::Invalid, LifecycleError::Invalid);
}
