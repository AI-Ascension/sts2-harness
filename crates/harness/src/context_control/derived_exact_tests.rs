// SPDX-License-Identifier: MIT

//! Fixtures for the derived-exact projection.
//!
//! Every expected value below is computed by hand in the test that asserts it, so a change in the
//! module cannot quietly redefine what "exact" means.

use super::*;
use serde_json::json;

/// Builds an observation with the given hand, energy, hit points, and enemies.
fn observation(energy: i64, hit_points: i64, hand: Value, enemies: Option<Value>) -> Value {
    let mut state = json!({"state": "combat", "turn_index": 1});
    if let (Some(enemies), Some(object)) = (enemies, state.as_object_mut()) {
        object.insert("enemies".to_owned(), enemies);
    }
    json!({
        "state_id": "combat-1",
        "generation": 7,
        "player": {"hp": hit_points, "max_hp": 80, "energy": energy, "gold": 99, "hand": hand},
        "state": state,
    })
}

/// Builds one enemy with an attacking intent.
fn attacker(enemy_id: &str, hit_points: i64, damage: i64, hits: Option<i64>) -> Value {
    let mut intent = json!({"kind": "attack", "damage": damage});
    if let (Some(hits), Some(object)) = (hits, intent.as_object_mut()) {
        object.insert("hits".to_owned(), json!(hits));
    }
    json!({"enemy_id": enemy_id, "name": "Jaw Worm", "hp": hit_points, "max_hp": 44, "intent": intent})
}

#[test]
fn sums_revealed_intent_damage_and_labels_survival() {
    // 8 + (3 x 4) = 20 gross against 30 hit points: at least half, below all of them.
    let facts = DerivedExactFacts::from_observation(&observation(
        3,
        30,
        json!([]),
        Some(json!([
            attacker("enemy-0", 12, 8, None),
            attacker("enemy-1", 20, 3, Some(4)),
        ])),
    ));
    assert!(facts.intents_revealed);
    assert_eq!(facts.incoming_damage_gross, Some(20));
    assert_eq!(facts.survival, Some(Survival::Heavy));
    assert_eq!(facts.enemy_count, 2);
    assert_eq!(facts.weakest_enemy_id.as_deref(), Some("enemy-0"));
}

#[test]
fn labels_a_total_at_or_above_hit_points_as_fatal_and_a_small_one_as_survivable() {
    let fatal = DerivedExactFacts::from_observation(&observation(
        3,
        20,
        json!([]),
        Some(json!([attacker("enemy-0", 12, 20, None)])),
    ));
    assert_eq!(fatal.survival, Some(Survival::Fatal));

    let survivable = DerivedExactFacts::from_observation(&observation(
        3,
        40,
        json!([]),
        Some(json!([attacker("enemy-0", 12, 19, None)])),
    ));
    assert_eq!(survivable.survival, Some(Survival::Survivable));
}

#[test]
fn a_non_attacking_intent_contributes_nothing_but_keeps_the_total_statable() {
    let facts = DerivedExactFacts::from_observation(&observation(
        3,
        30,
        json!([]),
        Some(json!([
            attacker("enemy-0", 12, 6, None),
            json!({"enemy_id": "enemy-1", "name": "Cultist", "hp": 20, "max_hp": 20,
                   "intent": {"kind": "buff"}}),
        ])),
    ));
    assert!(facts.intents_revealed);
    assert_eq!(facts.incoming_damage_gross, Some(6));
}

#[test]
fn an_unrevealed_intent_removes_the_total_instead_of_counting_as_zero() {
    let facts = DerivedExactFacts::from_observation(&observation(
        3,
        30,
        json!([]),
        Some(json!([
            attacker("enemy-0", 12, 6, None),
            json!({"enemy_id": "enemy-1", "name": "Cultist", "hp": 20, "max_hp": 20}),
        ])),
    ));
    assert!(!facts.intents_revealed);
    assert_eq!(facts.incoming_damage_gross, None);
    assert_eq!(facts.survival, None);
    assert_eq!(facts.enemy_count, 2);
}

#[test]
fn a_malformed_damage_or_hits_value_removes_the_total() {
    for intent in [
        json!({"kind": "attack", "damage": "six"}),
        json!({"kind": "attack", "damage": 6, "hits": -2}),
        json!({"kind": "attack", "damage": -6}),
    ] {
        let facts = DerivedExactFacts::from_observation(&observation(
            3,
            30,
            json!([]),
            Some(json!([
                json!({"enemy_id": "enemy-0", "name": "Slime", "hp": 9, "max_hp": 9,
                       "intent": intent}),
            ])),
        ));
        assert!(facts.intents_revealed);
        assert_eq!(facts.incoming_damage_gross, None);
    }
}

#[test]
fn classifies_affordable_and_variable_cost_cards() {
    let hand = json!([
        {"card_id": "strike-0", "name": "Strike", "cost": 1, "upgraded": false},
        {"card_id": "bash-0", "name": "Bash", "cost": 2, "upgraded": false},
        {"card_id": "heavy-0", "name": "Heavy Blade", "cost": 3, "upgraded": false},
        {"card_id": "whirl-0", "name": "Whirlwind", "cost": -1, "upgraded": false},
    ]);
    let facts = DerivedExactFacts::from_observation(&observation(2, 30, hand, None));
    assert_eq!(facts.affordable_card_ids, vec!["strike-0", "bash-0"]);
    assert_eq!(facts.variable_cost_card_ids, vec!["whirl-0"]);
    assert_eq!(facts.hand_size, 4);
}

#[test]
fn an_empty_hand_and_an_absent_enemy_list_claim_nothing() {
    let facts = DerivedExactFacts::from_observation(&observation(3, 30, json!([]), None));
    assert_eq!(facts.hand_size, 0);
    assert_eq!(facts.enemy_count, 0);
    assert!(facts.affordable_card_ids.is_empty());
    assert!(facts.variable_cost_card_ids.is_empty());
    assert_eq!(facts.incoming_damage_gross, None);
    assert_eq!(facts.survival, None);
    assert_eq!(facts.weakest_enemy_id, None);
    // Vacuously true: there is no enemy whose intent could be unrevealed.
    assert!(facts.intents_revealed);
}

#[test]
fn a_tie_on_the_lowest_hit_points_names_no_weakest_enemy() {
    let tied = DerivedExactFacts::from_observation(&observation(
        3,
        30,
        json!([]),
        Some(json!([
            attacker("enemy-0", 9, 1, None),
            attacker("enemy-1", 9, 1, None),
            attacker("enemy-2", 20, 1, None),
        ])),
    ));
    assert_eq!(tied.weakest_enemy_id, None);

    let broken = DerivedExactFacts::from_observation(&observation(
        3,
        30,
        json!([]),
        Some(json!([
            attacker("enemy-0", 9, 1, None),
            attacker("enemy-1", 9, 1, None),
            attacker("enemy-2", 4, 1, None),
        ])),
    ));
    assert_eq!(broken.weakest_enemy_id.as_deref(), Some("enemy-2"));
}

#[test]
fn counts_an_enemy_list_at_its_declared_bound() {
    let enemies: Vec<Value> = (0..MAX_ENEMIES)
        .map(|index| attacker(&format!("enemy-{index}"), 10, 1, None))
        .collect();
    let facts =
        DerivedExactFacts::from_observation(&observation(3, 80, json!([]), Some(json!(enemies))));
    assert_eq!(facts.enemy_count, MAX_ENEMIES);
    // Every enemy attacks for exactly one, so the gross total is the enemy count.
    assert_eq!(facts.incoming_damage_gross, Some(64));
    assert_eq!(facts.survival, Some(Survival::Heavy));
}

#[test]
fn omission_is_visible_in_the_serialized_projection() {
    let facts = DerivedExactFacts::from_observation(&observation(3, 30, json!([]), None));
    let serialized = serde_json::to_value(&facts).ok();
    assert_eq!(
        serialized,
        Some(json!({
            "intents_revealed": true,
            "affordable_card_ids": [],
            "variable_cost_card_ids": [],
            "hand_size": 0,
            "enemy_count": 0,
        }))
    );
}
