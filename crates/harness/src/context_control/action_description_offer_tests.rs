// SPDX-License-Identifier: MIT

//! Descriptions for what is offered rather than what is held: reward screens, offered cards, the
//! potions the player is carrying, and the contents a reward discloses before it is taken.

use super::describe_action;
use serde_json::{Value, json};

fn entry(action_id: &str, action: Value) -> Value {
    json!({"action_id": action_id, "action": action})
}

/// A reward screen whose offered set the host describes rather than merely naming.
fn described_offer() -> Value {
    json!({
        "player": {"hp": 70, "max_hp": 80, "energy": 3, "gold": 120,
                   "hand": [], "deck": [], "discard": [], "exhaust": []},
        "state": {
            "state": "selection",
            "choices": [
                {"choice_id": "card:21:Setup-Strike", "name": "Setup Strike", "cost": 1,
                 "upgraded": false, "rarity": "common",
                 "description": "Deal 7 damage. Draw 1 card."},
                {"choice_id": "card:22:Tremble", "name": "Tremble", "cost": 2,
                 "upgraded": false, "rarity": "uncommon",
                 "description": "Apply 3 Vulnerable to ALL enemies."},
            ],
        },
    })
}

#[test]
fn an_offered_card_is_described_when_the_host_describes_the_offer() {
    assert_eq!(
        describe_action(
            &entry(
                "select_card:123:card:22:Tremble",
                json!({"kind": "select_card", "card_id": "card:22:Tremble"})
            ),
            &described_offer()
        ),
        "choose Tremble [2 energy] (uncommon): Apply 3 Vulnerable to ALL enemies."
    );
}

#[test]
fn an_offered_card_the_host_only_names_reads_as_its_identifier() {
    // Every host today lists the offered set as bare identifiers, which is this case.
    let bare = json!({
        "player": {"hp": 70, "max_hp": 80, "energy": 3, "gold": 120,
                   "hand": [], "deck": [], "discard": [], "exhaust": []},
        "state": {"state": "selection", "choices": ["card:22:Tremble"]},
    });
    assert_eq!(
        describe_action(
            &entry(
                "select_card:123:card:22:Tremble",
                json!({"kind": "select_card", "card_id": "card:22:Tremble"})
            ),
            &bare
        ),
        "choose card:22:Tremble"
    );
}

#[test]
fn a_described_reward_is_named_in_the_take_clause() {
    let source = json!({
        "player": {"hp": 70, "max_hp": 80, "energy": 3, "gold": 120,
                   "hand": [], "deck": [], "discard": [], "exhaust": []},
        "state": {
            "state": "reward",
            "options": [{"choice_id": "reward:1:GoldReward", "name": "75 gold"}],
        },
    });
    assert_eq!(
        describe_action(
            &entry(
                "choose_reward:77:reward:1:GoldReward",
                json!({"kind": "choose_reward", "reward_id": "reward:1:GoldReward"})
            ),
            &source
        ),
        "take the reward 75 gold"
    );
}

#[test]
fn leaving_a_reward_screen_reads_as_words_rather_than_an_identifier() {
    let source = described_offer();
    assert_eq!(
        describe_action(&entry("proceed:77", json!({"kind": "proceed"})), &source),
        "proceed"
    );
    assert_eq!(
        describe_action(
            &entry("skip_reward:123", json!({"kind": "skip_reward"})),
            &source
        ),
        "skip the reward"
    );
}

/// A player carrying a described potion, with an enemy to aim it at.
fn carrying() -> Value {
    json!({
        "player": {
            "hp": 40, "max_hp": 80, "energy": 3, "gold": 10,
            "hand": [], "deck": [], "discard": [], "exhaust": [],
            "potion_slots": 3, "max_potion_slots": 3,
            "relics": [
                {"relic_id": "relic:1", "name": "Burning Blood",
                 "description": "At the end of combat, heal 6 HP."}
            ],
            "potions": [
                {"potion_id": "potion:1", "name": "Fire Potion", "slot": 0, "usable": true,
                 "target_mode": "enemy", "description": "Deal 20 damage to target enemy."},
                {"potion_id": "potion:2", "name": "Block Potion", "slot": 1, "usable": true,
                 "target_mode": "none"}
            ],
        },
        "state": {
            "state": "combat",
            "enemies": [{"enemy_id": "enemy:1", "name": "Nibbit", "hp": 32, "max_hp": 44}],
        },
    })
}

#[test]
fn a_potion_is_named_with_what_the_host_says_it_does() {
    assert_eq!(
        describe_action(
            &entry(
                "use_potion:5:potion:1:enemy:1",
                json!({"kind": "use_potion", "potion_id": "potion:1", "target_id": "enemy:1"})
            ),
            &carrying()
        ),
        "use Fire Potion: Deal 20 damage to target enemy. at Nibbit (32 hit points left)"
    );
}

#[test]
fn a_potion_without_host_text_is_still_named_rather_than_numbered() {
    assert_eq!(
        describe_action(
            &entry(
                "use_potion:5:potion:2:none",
                json!({"kind": "use_potion", "potion_id": "potion:2", "target_id": null})
            ),
            &carrying()
        ),
        "use Block Potion"
    );
    assert_eq!(
        describe_action(
            &entry(
                "discard_potion:5:potion:2",
                json!({"kind": "discard_potion", "potion_id": "potion:2"})
            ),
            &carrying()
        ),
        "discard Block Potion"
    );
}

#[test]
fn a_potion_the_player_is_not_carrying_leaves_the_identifier_alone() {
    assert_eq!(
        describe_action(
            &entry(
                "use_potion:5:potion:9:none",
                json!({"kind": "use_potion", "potion_id": "potion:9", "target_id": null})
            ),
            &carrying()
        ),
        "use_potion:5:potion:9:none"
    );
}

/// A reward screen that discloses what each reward would present next.
fn disclosed_reward() -> Value {
    json!({
        "player": {"hp": 70, "max_hp": 80, "energy": 3, "gold": 120,
                   "hand": [], "deck": [], "discard": [], "exhaust": []},
        "state": {
            "state": "reward",
            "options": [
                {"choice_id": "reward:5:CardReward", "name": "Card reward", "contents": [
                    {"choice_id": "card:21:Setup-Strike", "name": "Setup Strike", "cost": 1,
                     "rarity": "common", "description": "Deal 7 damage. Draw 1 card."},
                    {"choice_id": "card:23:Blood-Wall", "name": "Blood Wall", "cost": 2,
                     "upgraded": true, "rarity": "rare", "description": "Gain 12 Block."}
                ]},
                {"choice_id": "reward:1:GoldReward", "name": "75 gold"}
            ],
        },
    })
}

#[test]
fn a_reward_says_what_it_would_offer_before_it_is_taken() {
    // The whole point: the decision to open a card reward is made knowing what is inside it.
    assert_eq!(
        describe_action(
            &entry(
                "choose_reward:54:reward:5:CardReward",
                json!({"kind": "choose_reward", "reward_id": "reward:5:CardReward"})
            ),
            &disclosed_reward()
        ),
        concat!(
            "take the reward Card reward, offering ",
            "Setup Strike [1 energy] (common): Deal 7 damage. Draw 1 card.; ",
            "Blood Wall (upgraded) [2 energy] (rare): Gain 12 Block."
        )
    );
}

#[test]
fn a_reward_with_nothing_to_disclose_is_unchanged() {
    assert_eq!(
        describe_action(
            &entry(
                "choose_reward:54:reward:1:GoldReward",
                json!({"kind": "choose_reward", "reward_id": "reward:1:GoldReward"})
            ),
            &disclosed_reward()
        ),
        "take the reward 75 gold"
    );
}

#[test]
fn contents_the_host_only_names_are_carried_as_the_names_they_are() {
    let mut source = disclosed_reward();
    source["state"]["options"][0]["contents"] = json!(["card:21:Setup-Strike"]);
    assert_eq!(
        describe_action(
            &entry(
                "choose_reward:54:reward:5:CardReward",
                json!({"kind": "choose_reward", "reward_id": "reward:5:CardReward"})
            ),
            &source
        ),
        "take the reward Card reward, offering card:21:Setup-Strike"
    );
}
