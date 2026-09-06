// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use serde_json::json;

fn observation() -> Value {
    json!({"visible_seed":"TEST", "state":{"state":"combat","turn_index":1},
        "player":{"hp":73,"deck":[],"discard":[],
            "hand":[{"card_id":"card:1","name":"Defend","cost":1,"upgraded":false}],
            "exhaust":[{"card_id":"card:2","name":"Iron Wave","cost":1,"upgraded":false}]},
        "legal_actions":[]})
}

#[test]
fn cross_pile_identity_assignment_rebinds_payload_and_retains_lineage() {
    let source = observation();
    let mut current = source.clone();
    current["player"]["hand"][0]["card_id"] = json!("card:2");
    current["player"]["exhaust"][0]["card_id"] = json!("card:1");
    let bindings = CardBindings::default()
        .reconcile(&source, &current)
        .expect("unique cards");
    assert_eq!(
        bindings.translate(&json!({"kind":"play_card","card_id":"card:1", "target_id":"enemy:1"})),
        json!({"kind":"play_card","card_id":"card:2", "target_id":"enemy:1"})
    );
    assert!(bindings.reconcile(&source, &source).is_none());
    let mut later = source.clone();
    let card = later["player"]["hand"]
        .as_array_mut()
        .expect("hand")
        .remove(0);
    later["player"]["discard"] = json!([card]);
    let translated = bindings.translate(&later);
    assert!(bindings.reconcile(&later, &translated).is_some());
}

#[test]
fn changed_content_order_multiplicity_and_non_bijections_fail() {
    let source = observation();
    for case in ["hp", "cost", "upgrade", "name", "alias", "extra", "seed"] {
        let mut current = source.clone();
        match case {
            "hp" => current["player"]["hp"] = json!(72),
            "cost" => current["player"]["hand"][0]["cost"] = json!(0),
            "upgrade" => current["player"]["hand"][0]["upgraded"] = json!(true),
            "name" => current["player"]["hand"][0]["name"] = json!("Strike"),
            "alias" => current["player"]["exhaust"][0]["card_id"] = json!("card:1"),
            "extra" => current["player"]["discard"] = source["player"]["hand"].clone(),
            _ => current["visible_seed"] = json!("OTHER"),
        }
        assert!(
            CardBindings::default()
                .reconcile(&source, &current)
                .is_none(),
            "{case}"
        );
    }
    let mut source = source;
    source["player"]["hand"] = json!([
        {"card_id":"card:1","name":"Defend"}, {"card_id":"card:3","name":"Strike"}]);
    let mut current = source.clone();
    current["player"]["hand"]
        .as_array_mut()
        .expect("hand")
        .reverse();
    assert!(
        CardBindings::default()
            .reconcile(&source, &current)
            .is_none()
    );
}

#[test]
fn new_duplicate_card_rebinding_is_ambiguous_and_does_not_commit() {
    let mut source = observation();
    source["player"]["hand"] = json!([
        {"card_id":"card:3","name":"Defend"}, {"card_id":"card:4","name":"Defend"}]);
    let mut current = source.clone();
    current["player"]["hand"][0]["card_id"] = json!("card:5");
    let bindings = CardBindings::default();
    assert!(bindings.reconcile(&source, &current).is_none());
    assert!(bindings.0.is_empty());
    assert!(bindings.reconcile(&source, &source).is_some());
}
