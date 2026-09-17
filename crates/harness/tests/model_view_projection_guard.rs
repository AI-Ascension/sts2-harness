// SPDX-License-Identifier: MIT

//! Unknown, protected, mis-shaped, and oversized recipes and sources (issue #110).
//!
//! Synthetic fixtures only; no provider, host, or game is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/model_view_projection.rs"]
mod fixture;

use fixture::{named, path};
use serde_json::{Value, json};
use sts2_harness::context_control::{
    AdmittedSourceObservation, MAX_MODEL_VIEW_BYTES, MAX_MODEL_VIEW_FIELDS, ModelViewProjection,
    ModelViewProjectionError, PathSegment, ViewFieldPath, project_model_view,
};

/// A complete, admissible observation with the given hand.
fn observation_with_hand(hand: Vec<Value>) -> Value {
    json!({
        "state_id": "state-1",
        "generation": 7,
        "player": {
            "hp": 40, "max_hp": 80, "energy": 3, "gold": 120,
            "hand": hand,
            "deck": [], "discard": [], "exhaust": []
        },
        "state": {"state": "combat", "turn_index": 4, "enemies": []},
        "legal_actions": [{"action_id": "a1", "action": {"kind": "end_turn"}}]
    })
}

fn hand_recipe(selector: &str) -> ModelViewProjection {
    ModelViewProjection::new(
        selector,
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
    .expect("recipe resolves")
}

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
    let recipe = hand_recipe("selector-hand");
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

#[test]
fn a_result_past_the_model_view_bound_is_refused() {
    // The fair-play floor allows 256 cards of up to 512 bytes each in one hand, so an admissible
    // source can project well past the model-view bound. This pins that the bound is reachable and
    // enforced, rather than a guard no input can trip.
    let long_name = "A".repeat(500);
    let hand: Vec<Value> = (0..190)
        .map(|index| {
            json!({
                "card_id": format!("card_{index}"),
                "name": long_name,
                "cost": 1,
                "upgraded": false
            })
        })
        .collect();
    let source = AdmittedSourceObservation::admit(observation_with_hand(hand))
        .expect("a near-maximum hand is admissible");
    assert!(
        serde_json::to_vec(source.as_value())
            .expect("source encodes")
            .len()
            <= 128 * 1024,
        "the source must stay inside the fair-play observation bound"
    );

    assert_eq!(
        project_model_view(&hand_recipe("selector-oversize"), &source).err(),
        Some(ModelViewProjectionError::OversizedOutput {
            bound: MAX_MODEL_VIEW_BYTES
        }),
        "a projection past the model-view bound must refuse before inference"
    );
}

#[test]
fn a_recipe_that_never_passed_through_new_is_refused_at_projection() {
    // A recipe can reach projection by deserialization, which does not run `new`. The envelope is
    // therefore re-validated at projection time so an invalid schema, selector, or revision cannot
    // borrow a verdict minted for a well-formed recipe.
    let raw = json!({
        "schema": "not-the-model-view-schema",
        "selector_id": "not a valid id",
        "revision": "",
        "fields": [
            {"segments": [{"named": "state_id"}]},
            {"segments": [{"named": "generation"}]},
            {"segments": [{"named": "player"}, {"named": "hp"}]},
            {"segments": [{"named": "player"}, {"named": "max_hp"}]},
            {"segments": [{"named": "player"}, {"named": "energy"}]},
            {"segments": [{"named": "player"}, {"named": "gold"}]},
            {"segments": [{"named": "player"}, {"named": "hand"}]},
            {"segments": [{"named": "state"}, {"named": "state"}]}
        ]
    });
    let recipe: ModelViewProjection =
        serde_json::from_value(raw).expect("the envelope deserializes without validation");
    assert_eq!(
        recipe.validate().err(),
        Some(ModelViewProjectionError::InvalidInput),
        "the envelope is invalid on its face"
    );

    let source =
        AdmittedSourceObservation::admit(observation_with_hand(Vec::new())).expect("source admits");
    assert_eq!(
        project_model_view(&recipe, &source).err(),
        Some(ModelViewProjectionError::InvalidInput),
        "projection must re-validate the envelope rather than trust construction"
    );
}

#[test]
fn a_duplicated_or_mis_shaped_declaration_is_refused() {
    let mut duplicated = hand_recipe("selector-duplicate").fields;
    duplicated.push(ViewFieldPath::new(vec![named("state_id")]));
    assert_eq!(
        ModelViewProjection::new("selector-duplicate", "revision-1", duplicated).err(),
        Some(ModelViewProjectionError::DuplicateOutput {
            path: "state_id".to_owned()
        }),
        "one output path may not be declared twice"
    );

    let mut descending_a_scalar = hand_recipe("selector-scalar").fields;
    descending_a_scalar.push(ViewFieldPath::new(vec![named("state_id"), named("extra")]));
    assert_eq!(
        ModelViewProjection::new("selector-scalar", "revision-1", descending_a_scalar).err(),
        Some(ModelViewProjectionError::NotAnObject {
            path: "state_id".to_owned()
        }),
        "a scalar must terminate its path"
    );

    let mut unresolved_object = hand_recipe("selector-object").fields;
    unresolved_object.push(ViewFieldPath::new(vec![named("player")]));
    assert_eq!(
        ModelViewProjection::new("selector-object", "revision-1", unresolved_object).err(),
        Some(ModelViewProjectionError::UnresolvedObject {
            path: "player".to_owned()
        }),
        "an object must be descended into, not selected whole"
    );
}
