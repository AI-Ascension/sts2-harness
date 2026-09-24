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

/// One byte past the ceiling derives a value no acceptance predicate can admit, and the input
/// itself is still inside the manifest bound — which is what makes it a trap. This is the
/// regression: without the owner's guard the input validates and then can never settle.
#[test]
fn one_byte_past_the_ceiling_derives_an_illegal_identity() {
    let over = "e".repeat(MAX_DERIVABLE_EXECUTION_ID_BYTES + 1);
    assert!(
        types::id(&over),
        "the input is still accepted by the manifest predicate, which is what makes it a trap"
    );

    let illegal: Vec<&str> = [
        derived_ids::DERIVED_BINDING_PREFIX,
        derived_ids::DERIVED_PREPARED_PREFIX,
        derived_ids::DERIVED_PROVIDER_ATTEMPT_PREFIX,
    ]
    .into_iter()
    .filter(|prefix| !types::id(&format!("{prefix}{over}")))
    .collect();

    assert!(
        !illegal.is_empty(),
        "one past the ceiling must derive at least one illegal value"
    );
    assert_eq!(
        illegal.len(),
        2,
        "both 19-byte prefixes overflow at 110; only the 18-byte binding prefix still fits"
    );
    for prefix in illegal {
        assert_eq!(format!("{prefix}{over}").len(), MAX_ID_BYTES + 1);
    }
}

/// The binding prefix is one byte shorter, so on its own it would tolerate a 110-byte identity.
/// The overall ceiling is therefore the *minimum* over the prefixes, not the maximum — the owner
/// must satisfy the tightest derivation, and this test pins that distinction so a future edit
/// cannot relax the bound to the loosest one.
#[test]
fn the_ceiling_follows_the_widest_prefix_not_the_narrowest_requirement() {
    let at_ceiling = "e".repeat(MAX_DERIVABLE_EXECUTION_ID_BYTES);
    let binding = format!("{}{at_ceiling}", derived_ids::DERIVED_BINDING_PREFIX);
    assert_eq!(
        binding.len(),
        MAX_ID_BYTES - 1,
        "the binding derivation is not the binding constraint"
    );

    let prepared = format!("{}{at_ceiling}", derived_ids::DERIVED_PREPARED_PREFIX);
    let provider = format!(
        "{}{at_ceiling}",
        derived_ids::DERIVED_PROVIDER_ATTEMPT_PREFIX
    );
    assert_eq!(prepared.len(), MAX_ID_BYTES);
    assert_eq!(provider.len(), MAX_ID_BYTES);
    assert_eq!(
        MAX_DERIVABLE_EXECUTION_ID_BYTES,
        MAX_ID_BYTES - prepared.len() + at_ceiling.len(),
        "the ceiling is set by the widest derived value being exactly legal"
    );
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

/// Runs the real owner over one identity width after proving every other layer accepts the
/// request, so the answer is the owner's own. `None` means the owner completed the manifest, which
/// is only reachable by an identity whose every derived value is representable.
fn owner_answer(width: usize) -> Result<Option<LifecycleError>, String> {
    let mut fixture = Fixture::new();
    let identity = "e".repeat(width);
    let mut request = crate::parse_bridge_request(
        include_bytes!("../../../../protocol-artifact/exo-bridge-v1/golden/request.json"),
        MAX_INPUT_BYTES,
    )
    .expect("golden request");
    request.model_execution_id = identity.clone();
    let input = crate::encode_bridge_request("request-1", "turn-1", &request, MAX_INPUT_BYTES)
        .expect("envelope");
    let mut manifest = fixture.manifest.clone();
    manifest.execution_id = identity;
    manifest.input_digest = crate::sha256_hex(&input);
    manifest.input_length = input.len();
    if crate::exo_lifecycle::validation::input(&manifest, &input).is_err() {
        return Err(String::from(
            "the fixture must be admissible everywhere except the owner's derivation",
        ));
    }
    let mut owner = fixture.owner();
    match owner.prepare_one_shot_manifest(manifest, &input, "2030-01-01T00:00:00Z") {
        Err(error) => Ok(Some(error)),
        Ok(_) => Ok(None),
    }
}

/// The regression is one byte wide, so the two cases differ by exactly one byte: the ceiling
/// identity clears the owner's guard and is only then refused downstream, while one byte past it is
/// refused by the guard itself as the ordinary sanitized `Invalid`.
///
/// This fixture's broker is not the reviewed one-shot profile, so its admission gate refuses with
/// `Held` whatever reaches it. `Held` therefore marks "the guard let this through", which is what
/// separates the two widths by variant alone: without the guard, the past-ceiling identity would
/// report `Held` as well, and the prefix arithmetic in the other cases in this file proves that
/// derivation is the over-long one no later layer can accept.
#[test]
fn the_owner_refuses_only_the_identity_whose_derivation_overflows() -> Result<(), String> {
    let at_ceiling = owner_answer(MAX_DERIVABLE_EXECUTION_ID_BYTES)?;
    assert_eq!(
        at_ceiling,
        Some(LifecycleError::Held),
        "the ceiling identity must clear the guard and only then meet the broker"
    );
    let past_ceiling = owner_answer(MAX_DERIVABLE_EXECUTION_ID_BYTES + 1)?;
    assert_eq!(
        past_ceiling,
        Some(LifecycleError::Invalid),
        "one byte past the ceiling must be refused by the owner's guard"
    );
    Ok(())
}
