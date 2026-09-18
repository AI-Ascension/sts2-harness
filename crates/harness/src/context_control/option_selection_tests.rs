// SPDX-License-Identifier: MIT

//! Partition, safety, and determinism fixtures for option selection.

use super::*;
use serde_json::json;

/// Builds an observation carrying the given legal-action catalog.
fn observation(legal_actions: Value) -> Value {
    json!({
        "state_id": "combat-1",
        "generation": 3,
        "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
        "state": {"state": "combat", "turn_index": 2},
        "legal_actions": legal_actions,
    })
}

/// Builds one card play with an explicit card instance and target.
fn play(action_id: &str, card_id: &str, target_id: &str) -> Value {
    json!({
        "action_id": action_id,
        "action": {"kind": "play_card", "card_id": card_id, "target_id": target_id},
    })
}

/// Builds the turn-ending action.
fn end_turn() -> Value {
    json!({"action_id": "combat.end-turn", "action": {"kind": "end_turn"}})
}

/// Every catalog identifier, presented or withheld, exactly once.
///
/// The folded list is not read here: it is a second view of the same entries the withheld
/// list already names, so counting both would double every folded identifier.
fn partition_ids(selection: &OptionSelection) -> Vec<String> {
    let mut ids: Vec<String> = selection
        .presented
        .iter()
        .map(|option| option.action_id.clone())
        .chain(
            selection
                .withheld
                .iter()
                .map(|option| option.action_id.clone()),
        )
        .collect();
    ids.sort();
    ids
}

#[test]
fn folds_identical_card_plays_and_records_where_each_went() {
    let selection = OptionSelection::from_observation(
        &observation(json!([
            play("play:card-1", "card-1", "enemy-0"),
            play("play:card-2", "card-2", "enemy-0"),
            play("play:card-3", "card-3", "enemy-0"),
            end_turn(),
        ])),
        MAX_PRESENTED_OPTIONS,
    );
    assert_eq!(selection.mode, SelectionMode::Single);
    assert_eq!(
        selection
            .presented
            .iter()
            .map(|option| option.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["play:card-1", "combat.end-turn"]
    );
    assert_eq!(
        selection
            .presented
            .first()
            .map(|option| option.folded.clone()),
        Some(vec!["play:card-2".to_owned(), "play:card-3".to_owned()])
    );
    assert_eq!(
        selection.withheld,
        vec![
            WithheldOption {
                action_id: "play:card-2".to_owned(),
                reason: WithheldReason::Duplicate {
                    folded_into: "play:card-1".to_owned()
                },
            },
            WithheldOption {
                action_id: "play:card-3".to_owned(),
                reason: WithheldReason::Duplicate {
                    folded_into: "play:card-1".to_owned()
                },
            },
        ]
    );
}

#[test]
fn a_different_target_is_a_different_option() {
    let selection = OptionSelection::from_observation(
        &observation(json!([
            play("play:card-1:enemy-0", "card-1", "enemy-0"),
            play("play:card-1:enemy-1", "card-1", "enemy-1"),
            end_turn(),
        ])),
        MAX_PRESENTED_OPTIONS,
    );
    assert_eq!(selection.presented.len(), 3);
    assert!(selection.withheld.is_empty());
}

#[test]
fn presented_and_withheld_partition_the_catalog() {
    let catalog = json!([
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
        play("play:card-3", "card-3", "enemy-1"),
        json!({"action_id": "broken"}),
        end_turn(),
    ]);
    let selection = OptionSelection::from_observation(&observation(catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(
        partition_ids(&selection),
        vec![
            "broken",
            "combat.end-turn",
            "play:card-1",
            "play:card-2",
            "play:card-3",
        ]
    );
    assert!(selection.withheld.iter().any(|option| {
        option.action_id == "broken" && option.reason == WithheldReason::Malformed
    }));
}

#[test]
fn a_single_legal_action_is_forced_and_needs_no_question() {
    let selection =
        OptionSelection::from_observation(&observation(json!([end_turn()])), MAX_PRESENTED_OPTIONS);
    assert_eq!(selection.mode, SelectionMode::Forced);
    assert_eq!(
        selection
            .presented
            .first()
            .map(|option| option.action_id.as_str()),
        Some("combat.end-turn")
    );
}

#[test]
fn folding_that_would_leave_one_option_presents_the_catalog_unchanged() {
    // Three copies of one card aimed at one target fold to a single option, which is not a
    // question, so the untouched catalog is presented instead.
    let selection = OptionSelection::from_observation(
        &observation(json!([
            play("play:card-1", "card-1", "enemy-0"),
            play("play:card-2", "card-2", "enemy-0"),
            play("play:card-3", "card-3", "enemy-0"),
        ])),
        MAX_PRESENTED_OPTIONS,
    );
    assert_eq!(selection.presented.len(), 3);
    assert!(selection.withheld.is_empty());
}

#[test]
fn folding_a_repeated_turn_ending_action_keeps_the_option_itself() {
    let catalog = json!([
        end_turn(),
        json!({"action_id": "combat.end-turn-again", "action": {"kind": "end_turn"}}),
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
    ]);
    let selection = OptionSelection::from_observation(&observation(catalog), MAX_PRESENTED_OPTIONS);
    assert!(
        selection
            .presented
            .iter()
            .any(|option| option.kind == "end_turn")
    );
}

#[test]
fn a_catalog_above_the_bound_asks_in_two_stages() {
    let mut catalog: Vec<Value> = (0..30)
        .map(|index| {
            play(
                &format!("play:card-{index}"),
                &format!("card-{index}"),
                &format!("enemy-{index}"),
            )
        })
        .collect();
    catalog.push(end_turn());
    let selection =
        OptionSelection::from_observation(&observation(json!(catalog)), MAX_PRESENTED_OPTIONS);
    assert_eq!(selection.mode, SelectionMode::TwoStage);
    assert_eq!(selection.presented.len(), 31);
    assert_eq!(selection.classes(), vec!["play_card", "end_turn"]);
}

#[test]
fn selection_is_stable_for_a_fixed_catalog() {
    let catalog = json!([
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
        play("play:card-3", "card-3", "enemy-1"),
        end_turn(),
    ]);
    let first =
        OptionSelection::from_observation(&observation(catalog.clone()), MAX_PRESENTED_OPTIONS);
    let second = OptionSelection::from_observation(&observation(catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_value(&first).ok(),
        serde_json::to_value(&second).ok()
    );
}

#[test]
fn an_absent_catalog_presents_nothing_and_withholds_nothing() {
    let selection =
        OptionSelection::from_observation(&json!({"state_id": "s"}), MAX_PRESENTED_OPTIONS);
    assert!(selection.presented.is_empty());
    assert!(selection.withheld.is_empty());
    assert_eq!(selection.mode, SelectionMode::Single);
}
