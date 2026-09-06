// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use sts2_harness::{
    ActionKind, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeStage, ModelExecutionId,
};

fn observation(stage: &str, generation: u64, id: &str, character: &str) -> Value {
    json!({"state_id":format!("state-{generation}"),"generation":generation,
        "visible_seed":"REPLAY1","player":{"hp":80,"max_hp":80,"energy":3,"gold":99,
        "deck":[],"hand":[],"discard":[],"exhaust":[]},
        "state":{"state":stage,"characters":["ironclad"]},
        "legal_actions":[{"action_id":id,"action":{"kind":"start_run","character_id":character}}]})
}

fn rows() -> Vec<Value> {
    let mut terminal = observation("defeat", 3, "unused", "ironclad");
    terminal["state"] = json!({"state":"defeat","reason":null});
    terminal["player"]["hp"] = json!(0);
    terminal["legal_actions"] = json!([]);
    vec![
        json!({"event":"model_decision","action_id":"start-1",
            "observation":observation("setup",1,"start-1","ironclad")}),
        json!({"event":"action_receipt","action_id":"start-1","operation_id":"op-1","status":"Unknown"}),
        json!({"event":"operation_wait_completed","operation_id":"op-1","effect":"test_terminal",
            "observation":terminal}),
        json!({"event":"episode_complete","observation":terminal}),
        json!({"protocol":"runtime-v3-gameplay","status":"complete"}),
    ]
}

fn parse(rows: &[Value]) -> Result<ReplayTrace, String> {
    let text = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    ReplayTrace::parse(text.as_bytes())
}

fn input(value: Value, stage: EpisodeStage) -> DecisionInput {
    let generation = value["generation"].as_u64().expect("fixture generation");
    let state = value["state_id"]
        .as_str()
        .expect("fixture state")
        .to_owned();
    let actions = value["legal_actions"]
        .as_array()
        .expect("fixture catalog")
        .iter()
        .map(|row| {
            EpisodeLegalAction::new(
                row["action_id"].as_str().expect("fixture id"),
                ActionKind::StartRun,
            )
            .expect("fixture action")
        })
        .collect();
    DecisionInput::new(
        ModelExecutionId::new(1).expect("fixture execution"),
        EpisodeObservation::new(&state, generation, stage, true, false, true, value)
            .expect("fixture observation"),
        EpisodeLegalActionSet::new(state, generation, actions).expect("fixture catalog"),
        "Replay the seed",
        vec![],
    )
}

#[test]
fn replay_rebinds_only_identity_and_requires_settlement_before_terminal_verification() {
    let trace = parse(&rows()).expect("complete settled source");
    let mut source = ReplaySource::new(trace);
    let fresh = input(
        observation("setup", 20, "start-fresh", "ironclad"),
        EpisodeStage::Setup,
    );
    assert!(
        matches!(source.decide(&fresh), Ok(Decision::Action { action_id,.. }) if action_id=="start-fresh")
    );
    let mut terminal = rows()[3]["observation"].clone();
    terminal["generation"] = json!(22);
    terminal["state_id"] = json!("state-22");
    let terminal = input(terminal, EpisodeStage::Defeat).observation;
    assert!(source.finish(&terminal).is_err());
    source.action_completed(true);
    assert!(source.finish(&terminal).is_ok());
    assert!(source.model_execution_id().is_none());
}

#[test]
fn content_and_seed_divergence_prevent_any_replay_action() {
    for field in ["seed", "hp", "payload"] {
        let mut value = observation("setup", 20, "start-fresh", "ironclad");
        match field {
            "seed" => value["visible_seed"] = json!("OTHER"),
            "hp" => value["player"]["hp"] = json!(79),
            _ => value["legal_actions"][0]["action"]["character_id"] = json!("other"),
        }
        let mut source = ReplaySource::new(parse(&rows()).expect("fixture trace"));
        assert!(source.decide(&input(value, EpisodeStage::Setup)).is_err());
        assert!(!source.awaiting);
        assert_eq!(source.cursor, 0);
    }
}

#[test]
fn selection_catalog_order_is_not_pile_order_or_choice_identity() {
    let mut source = observation("selection", 1, "choose", "ironclad");
    source["state"] = json!({"state":"selection", "choices":["card:a", "card:b"]});
    source["player"]["hand"] = json!([{"card_id":"card:a"}, {"card_id":"card:b"}]);
    let mut reordered = source.clone();
    reordered["state"]["choices"] = json!(["card:b", "card:a"]);
    assert_eq!(canonical(&source), canonical(&reordered));
    for changed in [
        json!(["card:a"]),
        json!(["card:a", "card:c"]),
        json!(["card:a", "card:b", "card:b"]),
    ] {
        reordered["state"]["choices"] = changed;
        assert_ne!(canonical(&source), canonical(&reordered));
    }
    reordered = source.clone();
    reordered["player"]["hand"] = json!([{"card_id":"card:b"}, {"card_id":"card:a"}]);
    assert_ne!(canonical(&source), canonical(&reordered));
}

#[test]
fn duplicate_semantic_actions_and_unsettled_replay_are_rejected() {
    let mut value = observation("setup", 20, "start-fresh", "ironclad");
    let mut duplicate = value["legal_actions"][0].clone();
    duplicate["action_id"] = json!("start-alias");
    value["legal_actions"]
        .as_array_mut()
        .expect("fixture catalog")
        .push(duplicate);
    let mut source = ReplaySource::new(parse(&rows()).expect("fixture trace"));
    assert!(source.decide(&input(value, EpisodeStage::Setup)).is_err());
    let mut source = ReplaySource::new(parse(&rows()).expect("fixture trace"));
    let fresh = input(
        observation("setup", 20, "start-fresh", "ironclad"),
        EpisodeStage::Setup,
    );
    assert!(source.decide(&fresh).is_ok());
    source.action_completed(false);
    assert!(source.decide(&fresh).is_err());
    assert_eq!(source.cursor, 0);
}

#[test]
fn source_must_be_complete_seeded_and_have_matching_settlement() {
    for change in [
        "missing_terminal",
        "unsettled",
        "wrong_operation",
        "wrong_action",
        "seed",
        "resumed",
        "duplicate_terminal",
    ] {
        let mut value = rows();
        match change {
            "missing_terminal" => {
                value.truncate(3);
            }
            "unsettled" => {
                value.remove(2);
            }
            "wrong_operation" => value[2]["operation_id"] = json!("other"),
            "wrong_action" => value[1]["action_id"] = json!("other"),
            "seed" => value[0]["observation"]["visible_seed"] = Value::Null,
            "resumed" => value[0]["observation"]["state"]["state"] = json!("combat"),
            _ => value.push(value[3].clone()),
        }
        assert!(parse(&value).is_err(), "{change}");
    }
}

#[test]
fn terminal_content_is_checked_after_all_actions_settle() {
    let mut source = ReplaySource::new(parse(&rows()).expect("fixture trace"));
    let fresh = input(
        observation("setup", 20, "start-fresh", "ironclad"),
        EpisodeStage::Setup,
    );
    assert!(source.decide(&fresh).is_ok());
    source.action_completed(true);
    let mut terminal = rows()[3]["observation"].clone();
    terminal["player"]["gold"] = json!(100);
    assert!(
        source
            .finish(&input(terminal, EpisodeStage::Defeat).observation)
            .is_err()
    );
}

#[test]
fn prefix_requires_explicit_mode_and_stops_only_at_matching_settled_checkpoint() {
    let mut values = rows();
    values.truncate(3);
    values[2]["observation"] = observation("setup", 3, "next", "ironclad");
    let encode = |values: &[Value]| {
        values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let bytes = encode(&values);
    assert!(ReplayTrace::parse(bytes.as_bytes()).is_err());
    let trace = ReplayTrace::parse_mode(bytes.as_bytes(), true).expect("settled prefix");
    let mut source = ReplaySource::new(trace);
    let fresh = input(
        observation("setup", 20, "start-fresh", "ironclad"),
        EpisodeStage::Setup,
    );
    assert!(matches!(source.decide(&fresh), Ok(Decision::Action { .. })));
    source.action_completed(true);
    assert!(
        matches!(source.decide(&fresh), Ok(Decision::Recovery { kind, .. }) if kind == "stop_episode")
    );
    assert!(source.prefix_verified);
    values[2]["observation"]["legal_actions"] = json!([]);
    assert!(ReplayTrace::parse_mode(encode(&values).as_bytes(), true).is_err());
    values.truncate(2);
    assert!(ReplayTrace::parse_mode(encode(&values).as_bytes(), true).is_err());
}

#[test]
fn settled_receipt_can_complete_an_action_without_an_extra_wait() {
    let mut values = rows();
    values[1]["status"] = json!("Settled");
    values[1]["observation"] = values[2]["observation"].clone();
    values[1]["effect"] = json!("test_terminal");
    assert!(parse(&values).is_ok());
    values.remove(2);
    assert!(parse(&values).is_ok());
    values[1]["observation"]["player"]["gold"] = json!(1);
    assert!(parse(&values).is_err());
    values[1]["observation"] = Value::Null;
    assert!(parse(&values).is_err());
}

#[test]
fn source_rejects_reused_operation_identity_and_conflicting_settlement() {
    let mut values = rows();
    let mut next = values[0].clone();
    next["observation"] = values[2]["observation"].clone();
    next["observation"]["legal_actions"] = values[0]["observation"]["legal_actions"].clone();
    values.insert(3, next);
    values.insert(4, values[1].clone());
    values.insert(5, values[2].clone());
    assert!(parse(&values).is_err());
    let mut values = rows();
    values[1]["status"] = json!("Settled");
    values[1]["effect"] = json!("test_terminal");
    values[1]["observation"] = values[2]["observation"].clone();
    values[2]["observation"]["player"]["gold"] = json!(1);
    assert!(parse(&values).is_err());
}

#[test]
fn rejected_admission_is_skipped_but_uncertain_or_cross_seed_rejection_is_not() {
    let mut values = rows();
    let rejected = json!({"event":"action_receipt","action_id":"start-1",
        "operation_id":"not-dispatched","status":"Rejected","effect":null,
        "observation":values[0]["observation"]});
    values.insert(0, values[0].clone());
    values.insert(1, rejected);
    let parsed = parse(&values).expect("unchanged rejected admission");
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.rejected_attempts, 1);
    values[1]["observation"]["player"]["hp"] = json!(79);
    assert_eq!(
        parse(&values)
            .expect("asynchronous observation change")
            .rejected_attempts,
        1
    );
    values[1]["observation"]["visible_seed"] = json!("OTHER");
    assert!(parse(&values).is_err());
    values[1]["observation"]["visible_seed"] = json!("REPLAY1");
    values[1]["effect"] = json!("unexpected_effect");
    assert!(parse(&values).is_err());
    values[1]["effect"] = Value::Null;
    values[1]["observation"]["player"]["hp"] = json!(80);
    let mut unknown = values[1].clone();
    unknown["status"] = json!("Unknown");
    unknown["observation"] = Value::Null;
    values.insert(1, unknown);
    assert!(parse(&values).is_err());
}

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
