// SPDX-License-Identifier: MIT

//! Partition, fold, safety, and determinism fixtures for option selection.
//!
//! The fold cases come from the defect report on #302: a key that is not injective on the host's
//! action identity folds options that are genuinely different, so most of these tests assert that
//! something is **not** folded.

use super::*;
use serde_json::json;

/// Builds an observation carrying the given hand and legal-action catalog.
fn observation(hand: Value, legal_actions: Value) -> Value {
    json!({
        "state_id": "combat-1",
        "generation": 3,
        "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": hand},
        "state": {"state": "combat", "turn_index": 2},
        "legal_actions": legal_actions,
    })
}

/// One card in hand.
fn card(card_id: &str, name: &str, cost: i64) -> Value {
    json!({"card_id": card_id, "name": name, "cost": cost, "upgraded": false})
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

/// Three copies of one card, and the plays that name them.
fn three_copies() -> (Value, Value) {
    let hand = json!([
        card("card-1", "Strike", 1),
        card("card-2", "Strike", 1),
        card("card-3", "Strike", 1),
    ]);
    let catalog = json!([
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
        play("play:card-3", "card-3", "enemy-0"),
        end_turn(),
    ]);
    (hand, catalog)
}

/// Every catalog identifier, presented or withheld, exactly once.
///
/// The folded list is not read here: it is a second view of the same entries the withheld list
/// already names, so counting both would double every folded identifier.
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

/// The presented identifiers, in order.
fn presented(selection: &OptionSelection) -> Vec<String> {
    selection
        .presented
        .iter()
        .map(|option| option.action_id.clone())
        .collect()
}

#[test]
fn folds_repeated_copies_of_one_card_in_hand() {
    let (hand, catalog) = three_copies();
    let selection =
        OptionSelection::from_observation(&observation(hand, catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(selection.mode, SelectionMode::Single);
    assert_eq!(
        presented(&selection),
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
fn never_folds_two_different_cards_at_one_target() {
    // The defect: Strike and Bash aimed at one enemy differ only in `card_id`, which the old key
    // excluded, so one of them disappeared and was recorded as an intentional duplicate.
    let hand = json!([card("card-1", "Strike", 1), card("card-2", "Bash", 2)]);
    let catalog = json!([
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
        end_turn(),
    ]);
    let selection =
        OptionSelection::from_observation(&observation(hand, catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(
        presented(&selection),
        vec!["play:card-1", "play:card-2", "combat.end-turn"]
    );
    assert!(selection.withheld.is_empty());
}

#[test]
fn never_folds_the_same_card_name_at_a_different_cost_or_upgrade() {
    let hand = json!([
        card("card-1", "Strike", 1),
        card("card-2", "Strike", 0),
        json!({"card_id": "card-3", "name": "Strike", "cost": 1, "upgraded": true}),
    ]);
    let catalog = json!([
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
        play("play:card-3", "card-3", "enemy-0"),
        end_turn(),
    ]);
    let selection =
        OptionSelection::from_observation(&observation(hand, catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(selection.presented.len(), 4);
    assert!(selection.withheld.is_empty());
}

#[test]
fn never_folds_identities_the_vocabulary_does_not_declare() {
    // `potion_id`, `rest_option_id` and `selection_id` are emitted by the host codec but are absent
    // from the model-view field catalog. A key built from a known-field list drops them and folds
    // two genuinely different actions; a key built from the whole action keeps them apart.
    let catalog = json!([
        {"action_id": "potion:1", "action": {"kind": "use_potion", "potion_id": "fire", "target_id": "enemy-0"}},
        {"action_id": "potion:2", "action": {"kind": "use_potion", "potion_id": "block", "target_id": "enemy-0"}},
        {"action_id": "rest:heal", "action": {"kind": "rest_option", "rest_option_id": "heal"}},
        {"action_id": "rest:mend", "action": {"kind": "rest_option", "rest_option_id": "mend"}},
        {"action_id": "sel:1", "action": {"kind": "confirm_selection", "selection_id": "a"}},
        {"action_id": "sel:2", "action": {"kind": "confirm_selection", "selection_id": "b"}},
    ]);
    let selection =
        OptionSelection::from_observation(&observation(json!([]), catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(selection.presented.len(), 6);
    assert!(selection.withheld.is_empty());
}

#[test]
fn an_unresolvable_card_never_folds() {
    // Nothing in hand matches, so the instance identifier stays in the key and no fold is claimed.
    let catalog = json!([
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
        end_turn(),
    ]);
    let selection =
        OptionSelection::from_observation(&observation(json!([]), catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(selection.presented.len(), 3);
    assert!(selection.withheld.is_empty());
}

#[test]
fn a_different_target_is_a_different_option() {
    let hand = json!([card("card-1", "Strike", 1), card("card-2", "Strike", 1)]);
    let catalog = json!([
        play("play:card-1:enemy-0", "card-1", "enemy-0"),
        play("play:card-2:enemy-1", "card-2", "enemy-1"),
        end_turn(),
    ]);
    let selection =
        OptionSelection::from_observation(&observation(hand, catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(selection.presented.len(), 3);
    assert!(selection.withheld.is_empty());
}

#[test]
fn presented_and_withheld_partition_the_catalog() {
    let hand = json!([card("card-1", "Strike", 1), card("card-2", "Strike", 1)]);
    let catalog = json!([
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
        json!({"action_id": "broken"}),
        end_turn(),
    ]);
    let selection =
        OptionSelection::from_observation(&observation(hand, catalog), MAX_PRESENTED_OPTIONS);
    assert_eq!(
        partition_ids(&selection),
        vec!["broken", "combat.end-turn", "play:card-1", "play:card-2"]
    );
    assert!(
        selection.withheld.iter().any(
            |option| option.action_id == "broken" && option.reason == WithheldReason::Malformed
        )
    );
}

#[test]
fn a_single_legal_action_is_forced_and_needs_no_question() {
    let selection = OptionSelection::from_observation(
        &observation(json!([]), json!([end_turn()])),
        MAX_PRESENTED_OPTIONS,
    );
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
    let hand = json!([
        card("card-1", "Strike", 1),
        card("card-2", "Strike", 1),
        card("card-3", "Strike", 1),
    ]);
    let catalog = json!([
        play("play:card-1", "card-1", "enemy-0"),
        play("play:card-2", "card-2", "enemy-0"),
        play("play:card-3", "card-3", "enemy-0"),
    ]);
    let selection =
        OptionSelection::from_observation(&observation(hand, catalog), MAX_PRESENTED_OPTIONS);
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
    let selection =
        OptionSelection::from_observation(&observation(json!([]), catalog), MAX_PRESENTED_OPTIONS);
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
    let selection = OptionSelection::from_observation(
        &observation(json!([]), json!(catalog)),
        MAX_PRESENTED_OPTIONS,
    );
    assert_eq!(selection.mode, SelectionMode::TwoStage);
    assert_eq!(selection.presented.len(), 31);
    assert_eq!(selection.classes(), vec!["play_card", "end_turn"]);
}

#[test]
fn selection_is_stable_for_a_fixed_catalog() {
    let (hand, catalog) = three_copies();
    let first = OptionSelection::from_observation(
        &observation(hand.clone(), catalog.clone()),
        MAX_PRESENTED_OPTIONS,
    );
    let second =
        OptionSelection::from_observation(&observation(hand, catalog), MAX_PRESENTED_OPTIONS);
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
