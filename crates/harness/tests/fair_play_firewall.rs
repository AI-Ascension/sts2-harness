// SPDX-License-Identifier: MIT

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
    for key in ["raw_memory", "future_rng", "host_object", "private_prompt", "screen_coordinate"] {
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
    assert_eq!(SanitizedObservation::new(unknown), Err(SandboxError::UnknownField));

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
    assert_eq!(SanitizedObservation::new(numeric), Err(SandboxError::InvalidNumber));

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
