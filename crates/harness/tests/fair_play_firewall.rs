// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::json;
use sts2_harness::{SandboxError, SanitizedObservation};

fn observation() -> serde_json::Value {
    json!({
        "state_id": "combat-1",
        "generation": 0,
        "visible_seed": "visible-seed-only",
        "player": {"hp": 10, "max_hp": 10, "energy": 3, "gold": 0, "hand": [], "deck": [], "discard": [], "exhaust": []},
        "state": {"state": "combat", "turn_index": 1, "enemies": []},
        "legal_actions": [{"action_id": "combat.end-turn", "action": {"kind": "end_turn"}}]
    })
}

#[test]
fn privileged_fields_are_rejected_before_provider_use() {
    for key in [
        "raw_memory",
        "future_rng",
        "host_object",
        "private_prompt",
        "screen_coordinate",
    ] {
        let mut value = observation();
        value[key] = json!("forbidden");
        assert_eq!(
            SanitizedObservation::new(value),
            Err(SandboxError::PrivilegedField),
            "field {key} must remain outside the Exo projection"
        );
    }
}

#[test]
fn unknown_fields_and_non_integer_numbers_are_rejected() {
    let mut unknown = observation();
    unknown["debug_text"] = json!("not part of fair play");
    assert_eq!(
        SanitizedObservation::new(unknown),
        Err(SandboxError::UnknownField)
    );

    let mut non_integer = observation();
    non_integer["generation"] = json!(0.5);
    assert_eq!(
        SanitizedObservation::new(non_integer),
        Err(SandboxError::InvalidNumber)
    );
}

#[test]
fn collection_and_observation_bounds_are_enforced() {
    let cards = (0..257)
        .map(|index| {
            json!({
                "card_id": format!("card-{index}"),
                "name": "Card",
                "cost": 1,
                "upgraded": false
            })
        })
        .collect::<Vec<_>>();
    let mut value = observation();
    value["player"]["hand"] = json!(cards);
    assert_eq!(
        SanitizedObservation::new(value),
        Err(SandboxError::InvalidCollection)
    );

    let mut numeric = observation();
    numeric["player"]["hp"] = json!(65_536);
    assert_eq!(
        SanitizedObservation::new(numeric),
        Err(SandboxError::InvalidNumber)
    );

    let mut duplicate = observation();
    duplicate["legal_actions"] = json!([
        {"action_id":"combat.end-turn","action":{"kind":"end_turn"}},
        {"action_id":"combat.end-turn","action":{"kind":"end_turn"}}
    ]);
    assert_eq!(
        SanitizedObservation::new(duplicate),
        Err(SandboxError::DuplicateLegalAction)
    );
}

#[test]
fn scalar_fields_cannot_be_empty_objects_or_collections() {
    for path in [
        "/state_id",
        "/generation",
        "/visible_seed",
        "/player/hp",
        "/player/energy",
        "/state/turn_index",
        "/legal_actions/0/action_id",
    ] {
        for malformed in [json!({}), json!([]), json!(["unexpected"])] {
            let mut value = observation();
            if let Some(field) = value.pointer_mut(path) {
                *field = malformed;
            }
            assert!(
                SanitizedObservation::new(value).is_err(),
                "scalar field {path} must reject object/array values"
            );
        }
    }
    for field in ["card_id", "name", "cost", "upgraded"] {
        let mut value = observation();
        value["player"]["hand"] = json!([
            {"card_id":"card-1", "name":"Card", "cost":1, "upgraded":false}
        ]);
        value["player"]["hand"][0][field] = json!({});
        assert!(SanitizedObservation::new(value).is_err(), "card {field}");
    }
}

#[test]
fn collection_fields_require_flat_arrays_of_the_declared_item_type() {
    let card = json!({"card_id":"card-1", "name":"Card", "cost":1, "upgraded":false});
    for malformed in [card.clone(), json!([[card]]), json!([[]])] {
        let mut value = observation();
        value["player"]["hand"] = malformed;
        assert!(SanitizedObservation::new(value).is_err());
    }
    for malformed in [json!("node-1"), json!({}), json!([["node-1"]]), json!([{}])] {
        let mut value = observation();
        value["state"] = json!({"state":"map", "node_id":null, "options":malformed});
        assert!(SanitizedObservation::new(value).is_err());
    }
    let mut valid = observation();
    valid["visible_seed"] = json!(null);
    valid["state"] = json!({"state":"map", "node_id":null, "options":["node-1"]});
    assert!(SanitizedObservation::new(valid).is_ok());
}

#[test]
fn visible_seed_is_the_only_optional_root_field() {
    let mut without_seed = observation();
    without_seed
        .as_object_mut()
        .expect("fixture is an object")
        .remove("visible_seed");
    let projection = SanitizedObservation::new(without_seed).expect("seed is optional at the root");
    assert!(!projection.has_visible_seed());

    let with_seed = SanitizedObservation::new(observation()).expect("seed is admitted as text");
    assert!(with_seed.has_visible_seed());
    let stripped = with_seed.without_visible_seed();
    assert!(!stripped.has_visible_seed());
    assert!(stripped.as_value().get("visible_seed").is_none());
    assert_eq!(stripped, projection);

    for required in ["state_id", "generation", "player", "state", "legal_actions"] {
        let mut value = observation();
        value
            .as_object_mut()
            .expect("fixture is an object")
            .remove(required);
        assert_eq!(
            SanitizedObservation::new(value),
            Err(SandboxError::UnknownField),
            "root field {required} must stay required"
        );
    }
}
