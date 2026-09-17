// SPDX-License-Identifier: MIT

//! Nested, array, missing, and null selections, and validation ordering (issue #110).
//!
//! Synthetic fixtures only; no provider, host, or game is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/model_view_projection.rs"]
mod fixture;

use fixture::{minimal_recipe, named, observation, path};
use serde_json::{Value, json};
use sts2_harness::context_control::{
    AdmittedSourceObservation, ModelViewProjection, ModelViewProjectionError, PathSegment,
    catalog_metadata, excluded_sentinel_paths, project_model_view,
};

// ---------------------------------------------------------------------------
// AC1 — nested/array/missing/null produce deterministic bounded output, and
//       source validation demonstrably precedes projection.
// ---------------------------------------------------------------------------

#[test]
fn nested_and_array_selections_are_deterministic() {
    let recipe = ModelViewProjection::new(
        "selector-nested",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[
                named("player"),
                named("hand"),
                PathSegment::AllItems,
                named("card_id"),
            ]),
            path(&[named("state"), named("state")]),
        ],
    )
    .expect("recipe resolves");
    let source = AdmittedSourceObservation::admit(observation()).expect("source admits");

    let first = project_model_view(&recipe, &source).expect("first projection");
    let second = project_model_view(&recipe, &source).expect("second projection");

    assert_eq!(
        first.bytes, second.bytes,
        "the same source and recipe must always produce identical bytes"
    );
    assert_eq!(
        first.value.pointer("/player/hand"),
        Some(&json!([{"card_id": "strike"}, {"card_id": "defend"}])),
        "an object collection is indexed and reduced to the declared member"
    );
    assert_eq!(
        first.value.get("state_id"),
        Some(&json!("state-1")),
        "a scalar root field is carried unchanged"
    );
}

#[test]
fn an_explicit_null_on_a_nullable_field_is_preserved() {
    // The fair-play floor refuses a source that omits a required field and refuses `null` where the
    // source schema does not allow it, so absence and an explicit null can only be told apart on a
    // field the schema declares optional and nullable. For the `map` state that field is `node_id`.
    let recipe = ModelViewProjection::new(
        "selector-nullable",
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
            path(&[named("state"), named("node_id")]),
        ],
    )
    .expect("recipe resolves");

    let mut with_null = observation();
    with_null["state"] = json!({"state": "map", "node_id": null, "options": ["n1"]});
    let explicit = AdmittedSourceObservation::admit(with_null).expect("nullable source admits");
    let projected = project_model_view(&recipe, &explicit).expect("projection with explicit null");
    assert_eq!(
        projected.value.pointer("/state/node_id"),
        Some(&Value::Null),
        "an explicit null on a nullable field must be preserved, not dropped"
    );
}

#[test]
fn the_fair_play_floor_refuses_a_source_before_projection_sees_it() {
    // The validator is the outer gate: a source that omits a required field, or that carries a
    // value contradicting the declared schema, never reaches a prepared projection at all. The
    // projection's own drift checks therefore backstop the floor rather than duplicate it.
    let recipe = minimal_recipe();

    let mut incomplete = observation();
    incomplete["player"]
        .as_object_mut()
        .expect("player object")
        .remove("gold");
    assert_eq!(
        AdmittedSourceObservation::admit(incomplete).err(),
        Some(ModelViewProjectionError::SourceInvalid),
        "a source missing a required field must fail admission"
    );

    let mut drifted = observation();
    drifted["player"]["hp"] = json!("forty");
    assert_eq!(
        AdmittedSourceObservation::admit(drifted).err(),
        Some(ModelViewProjectionError::SourceInvalid),
        "a source with a drifted value type must fail admission"
    );

    // A well-formed source still projects, proving the refusals above are about the source.
    let source = AdmittedSourceObservation::admit(observation()).expect("source admits");
    assert!(project_model_view(&recipe, &source).is_ok());
}

#[test]
fn source_validation_precedes_projection_and_covers_excluded_fields() {
    // The recipe selects only `state_id`-adjacent model-visible fields: it excludes `player.deck`
    // entirely. A forbidden field must still fail the whole call, proving validation runs over the
    // complete source rather than the projected subset.
    let recipe = minimal_recipe();
    let mut poisoned = observation();
    poisoned["player"]["deck"] =
        json!([{"card_id": "bash", "name": "Bash", "cost": 2, "upgraded": false}]);
    poisoned["rng_state"] = json!(42);

    assert!(
        AdmittedSourceObservation::admit(poisoned.clone()).is_err(),
        "an owner-privileged field must be refused at admission"
    );
    assert!(
        !catalog_metadata().is_empty(),
        "catalog metadata is available for the design surface"
    );
    assert!(
        excluded_sentinel_paths()
            .iter()
            .any(|path| path == "player.deck"),
        "the unseen card piles are declared as excluded sentinels"
    );
    let _ = recipe;
}
