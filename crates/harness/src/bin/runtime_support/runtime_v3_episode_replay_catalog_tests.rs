// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{Decision, EpisodeStage};

use super::*;

#[test]
fn asynchronous_boundary_waits_never_dispatch_divergent_actions_and_are_bounded() {
    let make_source = || {
        let mut source = ReplaySource::new(parse(&rows()).expect("fixture source"));
        let mut next = observation("combat", 2, "next-source", "ironclad");
        next["state"] = json!({"state":"combat","turn_index":1,"enemies":[]});
        source.trace.records.push(trace::ReplayRecord {
            action_id: "next-source".into(),
            payload: next["legal_actions"][0]["action"].clone(),
            observation: next,
        });
        let first = input(
            observation("setup", 20, "first", "ironclad"),
            EpisodeStage::Setup,
        );
        assert!(matches!(source.decide(&first), Ok(Decision::Action { .. })));
        source.action_completed(true);
        source
    };
    let mut source = make_source();
    let transient = input(
        observation("setup", 21, "transient", "ironclad"),
        EpisodeStage::Setup,
    );
    assert!(matches!(
        source.decide(&transient),
        Ok(Decision::Wait { .. })
    ));
    assert!(!source.awaiting);
    assert_eq!(source.cursor, 1);
    let mut matched = observation("combat", 22, "current", "ironclad");
    matched["state"] = json!({"state":"combat","turn_index":1,"enemies":[]});
    let matched = input(matched, EpisodeStage::Combat);
    assert!(
        matches!(source.decide(&matched), Ok(Decision::Action { action_id, .. }) if action_id == "current")
    );

    let mut source = make_source();
    for _ in 0..3 {
        assert!(matches!(
            source.decide(&transient),
            Ok(Decision::Wait { .. })
        ));
        assert!(!source.awaiting);
    }
    assert!(source.decide(&transient).is_err());
    let mut source = make_source();
    let mut different_seed = observation("setup", 21, "transient", "ironclad");
    different_seed["visible_seed"] = json!("OTHER");
    assert!(
        source
            .decide(&input(different_seed, EpisodeStage::Setup))
            .is_err()
    );
}

#[test]
fn rebound_card_dispatch_requires_unique_current_payload_and_preserves_target() {
    let mut recorded = observation("combat", 1, "old", "ironclad");
    recorded["state"] = json!({"state":"combat","turn_index":1,"enemies":[]});
    recorded["player"]["hand"] = json!([
        {"card_id":"card:1","name":"Strike","cost":1,"upgraded":false}]);
    recorded["legal_actions"] = json!([{"action_id":"old",
        "action":{"kind":"play_card","card_id":"card:1","target_id":"enemy:1"}}]);
    let mut current = recorded.clone();
    current["player"]["hand"][0]["card_id"] = json!("card:2");
    current["legal_actions"][0]["action_id"] = json!("fresh");
    current["legal_actions"][0]["action"]["card_id"] = json!("card:2");
    for correct_target in [false, true] {
        let mut source = ReplaySource::new(parse(&rows()).expect("fixture trace"));
        source.trace.records[0] = trace::ReplayRecord {
            payload: recorded["legal_actions"][0]["action"].clone(),
            observation: recorded.clone(),
            action_id: "old".into(),
        };
        current["legal_actions"][0]["action"]["target_id"] =
            json!(if correct_target { "enemy:1" } else { "enemy:2" });
        let decision = source.decide(&input(current.clone(), EpisodeStage::Combat));
        if correct_target {
            assert!(
                matches!(decision, Ok(Decision::Action { action_id, .. }) if action_id == "fresh")
            );
        } else {
            assert!(decision.is_err());
            assert!(!source.awaiting);
        }
    }
}

#[test]
fn labeled_card_selection_dispatches_current_catalog_identity() {
    let mut recorded = observation("selection", 1, "old", "ironclad");
    recorded["player"]["hand"] = json!([
        {"card_id":"card:1","name":"Defend","cost":1,"upgraded":false}]);
    recorded["state"] = json!({"state":"selection","choices":["card:1:Defend"]});
    recorded["legal_actions"] = json!([{"action_id":"old",
        "action":{"kind":"select_card","card_id":"card:1:Defend"}}]);
    let mut current = recorded.clone();
    current["player"]["hand"][0]["card_id"] = json!("card:2");
    current["state"]["choices"] = json!(["card:2:Defend"]);
    current["legal_actions"] = json!([{"action_id":"fresh",
        "action":{"kind":"select_card","card_id":"card:2:Defend"}}]);
    let mut source = ReplaySource::new(parse(&rows()).expect("fixture trace"));
    source.trace.records[0] = trace::ReplayRecord {
        payload: recorded["legal_actions"][0]["action"].clone(),
        observation: recorded,
        action_id: "old".into(),
    };
    assert!(
        matches!(source.decide(&input(current, EpisodeStage::Selection)),
        Ok(Decision::Action { action_id, .. }) if action_id == "fresh")
    );
}
