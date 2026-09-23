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

#[test]
fn a_host_that_knows_its_own_card_text_may_send_it() {
    // require_exact counts keys, so admitting `description` in the field list alone still refused
    // the card for having five of them. An optional field has to be optional in the shape too.
    let mut value = observation();
    value["player"]["hand"] = json!([{
        "card_id": "card-1", "name": "Strike", "cost": 1, "upgraded": false,
        "description": "Deal 6 damage."
    }]);
    assert!(SanitizedObservation::new(value).is_ok());
}

#[test]
fn a_card_still_requires_the_fields_it_always_required() {
    let mut value = observation();
    value["player"]["hand"] = json!([{"card_id": "card-1", "description": "Deal 6 damage."}]);
    assert_eq!(
        SanitizedObservation::new(value).err(),
        Some(SandboxError::UnknownField)
    );
}

#[test]
fn a_malformed_continuation_projection_is_refused() {
    // The projection refuses a payload that does not carry the agreed continuation shape rather
    // than dropping the action, so a malformed host offer fails the whole observation loudly.
    for action in [
        json!("continue_run"),
        json!({"run_id": "profile1"}),
        json!({"kind": 7}),
        json!({"kind": "resume_run"}),
    ] {
        let mut value = observation();
        value["legal_actions"] = json!([{"action_id": "continue_run:2", "action": action}]);
        assert!(
            SanitizedObservation::new(value).is_err(),
            "the projection admitted a malformed continuation {action}"
        );
    }
}

#[test]
fn a_continuation_discriminator_is_an_opaque_host_identity() {
    // The host owns the run discriminator, and the projection admits only a plain identity. A value
    // that tries to name another profile's storage or carries a structured selector is refused
    // here, because the harness never resolves a profile or a save path itself.
    for (action, expected) in [
        (
            json!({"kind": "continue_run", "run_id": "profile2\\saves\\current_run.save"}),
            SandboxError::InvalidText,
        ),
        (
            json!({"kind": "continue_run", "run_id": {"profile": "profile2"}}),
            SandboxError::NotAnObservation,
        ),
        (
            json!({"kind": "continue_run", "run_id": ""}),
            SandboxError::InvalidText,
        ),
        (
            json!({"kind": "continue_run", "profile": "profile2"}),
            SandboxError::UnknownField,
        ),
        (
            json!({"kind": "continue_run", "run_id": "profile2", "profile_path": "a/save"}),
            SandboxError::UnknownField,
        ),
    ] {
        let mut value = observation();
        value["legal_actions"] =
            json!([{"action_id": "continue_run:2:profile2", "action": action}]);
        assert_eq!(SanitizedObservation::new(value), Err(expected));
    }
}

#[test]
fn an_offered_set_is_admitted_named_or_described() {
    // Every host today lists the offered set as identifiers, and that must keep working.
    let mut named = observation();
    named["state"] = json!({"state": "selection", "choices": ["card:22:Tremble"]});
    assert!(SanitizedObservation::new(named).is_ok());

    let mut described = observation();
    described["state"] = json!({
        "state": "selection",
        "choices": [{
            "choice_id": "card:22:Tremble", "name": "Tremble", "cost": 2,
            "upgraded": false, "rarity": "uncommon",
            "description": "Apply 3 Vulnerable to ALL enemies."
        }],
    });
    assert!(SanitizedObservation::new(described).is_ok());

    // The identity is what an action references, so an entry without one cannot be resolved.
    let mut anonymous = observation();
    anonymous["state"] = json!({"state": "selection", "choices": [{"name": "Tremble"}]});
    assert_eq!(
        SanitizedObservation::new(anonymous).err(),
        Some(SandboxError::UnknownField)
    );

    let mut unknown = observation();
    unknown["state"] = json!({
        "state": "selection",
        "choices": [{"choice_id": "card:22:Tremble", "win_probability": "high"}],
    });
    assert_eq!(
        SanitizedObservation::new(unknown).err(),
        Some(SandboxError::UnknownField)
    );
}

#[test]
fn what_the_player_is_carrying_is_admitted_with_what_it_does() {
    let mut value = observation();
    value["player"]["relics"] = json!([{
        "relic_id": "relic:1", "name": "Burning Blood",
        "description": "At the end of combat, heal 6 HP."
    }]);
    value["player"]["potions"] = json!([{
        "potion_id": "potion:1", "name": "Fire Potion", "slot": 0, "usable": true,
        "target_mode": "enemy", "description": "Deal 20 damage to target enemy."
    }]);
    value["player"]["potion_slots"] = json!(3);
    value["player"]["max_potion_slots"] = json!(3);
    value["legal_actions"] = json!([{
        "action_id": "use_potion:1:potion:1:enemy:1",
        "action": {"kind": "use_potion", "potion_id": "potion:1", "target_id": null}
    }]);
    assert!(SanitizedObservation::new(value).is_ok());
}

#[test]
fn a_player_carrying_nothing_is_unchanged() {
    // relics and potions are optional, so every host that sent neither still validates.
    assert!(SanitizedObservation::new(observation()).is_ok());
}

#[test]
fn a_host_may_offer_a_saved_run_to_continue() {
    // The host names a run only when the choice is not already determined by the screen, so both
    // the bare kind and the discriminated form must reach the provider projection.
    for action in [
        json!({"kind": "continue_run"}),
        json!({"kind": "continue_run", "run_id": "profile1"}),
    ] {
        let mut value = observation();
        value["legal_actions"] = json!([{"action_id": "continue_run:2", "action": action}]);
        assert!(SanitizedObservation::new(value).is_ok());
    }
}

#[test]
fn a_continuation_projection_carries_only_the_agreed_shape() {
    for (action, expected) in [
        (
            json!({"kind": "continue_run", "save_path": "profile1/saves/current_run.save"}),
            SandboxError::UnknownField,
        ),
        (
            json!({"kind": "continue_run", "run_id": "profile1", "seed": "1"}),
            SandboxError::UnknownField,
        ),
        (
            json!({"kind": "continue_run", "run_id": null}),
            SandboxError::NotAnObservation,
        ),
    ] {
        let mut value = observation();
        value["legal_actions"] = json!([{"action_id": "continue_run:2", "action": action}]);
        assert_eq!(SanitizedObservation::new(value), Err(expected));
    }

    // A kind the host does not own is still refused rather than ignored.
    let mut value = observation();
    value["legal_actions"] = json!([{"action_id": "resume:2", "action": {"kind": "resume_run"}}]);
    assert_eq!(
        SanitizedObservation::new(value),
        Err(SandboxError::UnknownField)
    );
}

#[test]
fn a_relic_or_potion_without_an_identity_or_name_is_refused() {
    for holding in [
        json!({"player_relics": [{"name": "Burning Blood"}]}),
        json!({"player_potions": [{"potion_id": "potion:1"}]}),
    ] {
        let mut value = observation();
        if let Some(relics) = holding.get("player_relics") {
            value["player"]["relics"] = relics.clone();
        } else {
            value["player"]["potions"] = holding["player_potions"].clone();
        }
        assert_eq!(
            SanitizedObservation::new(value).err(),
            Some(SandboxError::UnknownField)
        );
    }
}

#[test]
fn a_reward_may_disclose_what_it_would_offer_next() {
    let mut value = observation();
    value["state"] = json!({
        "state": "reward",
        "options": [{
            "choice_id": "reward:5:CardReward", "name": "Card reward",
            "contents": [{
                "choice_id": "card:21:Setup-Strike", "name": "Setup Strike", "cost": 1,
                "upgraded": false, "rarity": "common",
                "description": "Deal 7 damage. Draw 1 card."
            }],
        }],
    });
    assert!(SanitizedObservation::new(value).is_ok());
}

#[test]
fn disclosure_is_one_level_deep() {
    // An entry inside `contents` has no `contents` of its own, so a host cannot nest an observation
    // inside an observation and the projection needs no depth counter to stay bounded.
    let mut value = observation();
    value["state"] = json!({
        "state": "reward",
        "options": [{
            "choice_id": "reward:5:CardReward",
            "contents": [{
                "choice_id": "card:21:Setup-Strike",
                "contents": [{"choice_id": "card:99:Deeper"}]
            }],
        }],
    });
    assert_eq!(
        SanitizedObservation::new(value).err(),
        Some(SandboxError::UnknownField)
    );
}
