// SPDX-License-Identifier: MIT

//! Unknown, protected, mis-shaped, and oversized recipes and sources (issue #110).
//!
//! Synthetic fixtures only; no provider, host, or game is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/model_view_projection.rs"]
mod fixture;

use fixture::{named, path};
use sts2_harness::context_control::{
    MAX_MODEL_VIEW_FIELDS, ModelViewProjection, ModelViewProjectionError, PathSegment,
};

// ---------------------------------------------------------------------------
// AC2 — unknown/protected paths, schema drift, hidden source fields and
//       oversized results reject before inference.
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_path_is_refused_at_construction() {
    let unknown = ModelViewProjection::new(
        "selector-unknown",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[named("player"), named("hand")]),
            path(&[named("state"), named("state")]),
            path(&[named("player"), named("future_draw")]),
        ],
    );
    assert_eq!(
        unknown,
        Err(ModelViewProjectionError::UnknownPath {
            path: "player.future_draw".to_owned()
        })
    );
}

#[test]
fn a_protected_path_is_refused_at_construction() {
    for protected in [
        "legal_actions",
        "player.deck",
        "player.discard",
        "player.exhaust",
    ] {
        let segments: Vec<PathSegment> = protected.split('.').map(named).collect();
        let error = ModelViewProjection::new(
            "selector-protected",
            "revision-1",
            vec![
                path(&[named("state_id")]),
                path(&[named("generation")]),
                path(&[named("player"), named("hp")]),
                path(&[named("player"), named("max_hp")]),
                path(&[named("player"), named("energy")]),
                path(&[named("player"), named("gold")]),
                path(&[named("player"), named("hand")]),
                path(&[named("state"), named("state")]),
                path(&segments),
            ],
        )
        .expect_err("owner-only path must be refused");
        assert_eq!(
            error,
            ModelViewProjectionError::ProtectedPath {
                path: protected.to_owned()
            },
            "{protected} must be reported as owner-only"
        );
    }
}

#[test]
fn schema_drift_and_omitted_required_fields_are_refused() {
    let omitted = ModelViewProjection::new(
        "selector-omits-player",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("state"), named("state")]),
        ],
    );
    assert_eq!(
        omitted,
        Err(ModelViewProjectionError::RequiredFieldOmitted {
            field: "player".to_owned()
        }),
        "a recipe cannot silently drop a required model-visible root field"
    );

    // A recipe whose declared context no longer matches the source schema is caught by the
    // catalog resolver, not by a runtime walk.
    let unknown_context = ModelViewProjection::new(
        "selector-unknown-context",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[named("player"), named("hand")]),
            path(&[named("state"), named("state")]),
            path(&[named("state"), named("price")]),
        ],
    );
    assert_eq!(
        unknown_context,
        Err(ModelViewProjectionError::UnknownPath {
            path: "state.price".to_owned()
        }),
        "a member that belongs to another context is not reachable by name alone"
    );
}

#[test]
fn an_unindexed_object_collection_is_refused() {
    let unindexed = ModelViewProjection::new(
        "selector-unindexed",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[named("player"), named("hand"), named("card_id")]),
            path(&[named("state"), named("state")]),
        ],
    );
    assert_eq!(
        unindexed,
        Err(ModelViewProjectionError::MissingIndex {
            path: "player.hand".to_owned()
        }),
        "an object collection needs its explicit index before a member"
    );
}

#[test]
fn an_oversized_and_overlong_recipe_is_refused() {
    let oversized_bound = ModelViewProjection::new(
        "selector-too-many",
        "revision-1",
        (0..=MAX_MODEL_VIEW_FIELDS)
            .map(|_| path(&[named("state_id")]))
            .collect(),
    );
    assert!(matches!(
        oversized_bound,
        Err(ModelViewProjectionError::TooManyFields { .. })
    ));

    let too_long = ModelViewProjection::new(
        "selector-too-long",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[named("player"), named("hand")]),
            path(&[named("state"), named("state")]),
            path(&[
                named("player"),
                named("hand"),
                PathSegment::AllItems,
                named("card_id"),
                named("extra"),
            ]),
        ],
    );
    assert!(matches!(
        too_long,
        Err(ModelViewProjectionError::PathTooLong { .. })
    ));
}

#[test]
fn a_collection_beyond_its_declared_bound_is_refused() {
    let recipe = ModelViewProjection::new(
        "selector-hand",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[named("player"), named("hand")]),
            path(&[named("state"), named("state")]),
        ],
    )
    .expect("recipe resolves");
    assert_eq!(
        recipe.declared_paths(),
        vec![
            "state_id",
            "generation",
            "player.hp",
            "player.max_hp",
            "player.energy",
            "player.gold",
            "player.hand",
            "state.state",
        ]
    );
}
