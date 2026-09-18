// SPDX-License-Identifier: MIT

use super::describe_action;
use serde_json::{Value, json};

fn observation() -> Value {
    json!({
        "player": {
            "hp": 80, "max_hp": 80, "energy": 3, "gold": 99,
            "hand": [
                {"card_id": "card:11", "name": "Strike", "cost": 1, "upgraded": false},
                {"card_id": "card:12", "name": "Defend", "cost": 1, "upgraded": false},
                {"card_id": "card:16", "name": "Bash", "cost": 2, "upgraded": true},
                {"card_id": "card:17", "name": "Ritual", "cost": -1, "upgraded": false}
            ],
            "deck": [{"card_id": "card:1", "name": "Strike", "cost": 1, "upgraded": false}],
            "discard": [], "exhaust": []
        },
        "state": {
            "state": "combat",
            "enemies": [{"enemy_id": "enemy:1", "name": "Nibbit", "hp": 32, "max_hp": 44}],
            "items": [{"item_id": "item:3", "name": "Bag of Marbles", "price": 140}]
        }
    })
}

fn entry(action_id: &str, action: Value) -> Value {
    json!({"action_id": action_id, "action": action})
}

#[test]
fn a_played_card_is_named_with_its_cost_and_its_target() {
    assert_eq!(
        describe_action(
            &entry(
                "play:13:card:11:enemy:1",
                json!({"kind": "play_card", "card_id": "card:11", "target_id": "enemy:1"})
            ),
            &observation()
        ),
        "play Strike [1 energy] at Nibbit (32 hit points left)"
    );
}

#[test]
fn an_untargeted_card_carries_no_target_clause() {
    assert_eq!(
        describe_action(
            &entry(
                "play:13:card:12:none",
                json!({"kind": "play_card", "card_id": "card:12", "target_id": Value::Null})
            ),
            &observation()
        ),
        "play Defend [1 energy]"
    );
}

#[test]
fn an_upgrade_is_stated_because_the_host_states_it() {
    assert!(
        describe_action(
            &entry(
                "play:13:card:16:enemy:1",
                json!({"kind": "play_card", "card_id": "card:16", "target_id": "enemy:1"})
            ),
            &observation()
        )
        .starts_with("play Bash (upgraded) [2 energy]")
    );
}

#[test]
fn a_negative_cost_is_omitted_rather_than_printed_as_a_number() {
    // The host uses a negative cost to say the cost is not fixed; printing it would be a claim.
    let text = describe_action(
        &entry(
            "play:13:card:17:none",
            json!({"kind": "play_card", "card_id": "card:17"}),
        ),
        &observation(),
    );
    assert_eq!(text, "play Ritual");
    assert!(!text.contains("-1"));
}

#[test]
fn host_card_text_is_carried_when_the_host_supplies_it() {
    let mut source = observation();
    source["player"]["hand"][0]["description"] = json!("Deal 6 damage.");
    assert_eq!(
        describe_action(
            &entry(
                "play:13:card:11:enemy:1",
                json!({"kind": "play_card", "card_id": "card:11", "target_id": "enemy:1"})
            ),
            &source
        ),
        "play Strike [1 energy]: Deal 6 damage. at Nibbit (32 hit points left)"
    );
}

#[test]
fn a_card_outside_the_hand_is_still_found_in_the_deck() {
    assert_eq!(
        describe_action(
            &entry(
                "remove:1:card:1",
                json!({"kind": "shop_remove", "card_id": "card:1"})
            ),
            &observation()
        ),
        "remove Strike [1 energy] from the deck"
    );
}

#[test]
fn the_simple_kinds_read_as_themselves() {
    let source = observation();
    for (action, expected) in [
        (json!({"kind": "end_turn"}), "end the turn"),
        (
            json!({"kind": "confirm_selection"}),
            "confirm the selection",
        ),
        (json!({"kind": "cancel_selection"}), "cancel the selection"),
        (
            json!({"kind": "start_run", "character_id": "IRONCLAD"}),
            "start a run as IRONCLAD",
        ),
        (
            json!({"kind": "select_map_node", "node_id": "map:0:0:3"}),
            "travel to map node map:0:0:3",
        ),
        (
            json!({"kind": "choose_reward", "reward_id": "reward:5"}),
            "take the reward reward:5",
        ),
        (
            json!({"kind": "event_choice", "choice_id": "choice:2"}),
            "choose the option choice:2",
        ),
        (
            json!({"kind": "shop_purchase", "item_id": "item:3"}),
            "buy Bag of Marbles for 140 gold",
        ),
    ] {
        assert_eq!(describe_action(&entry("id", action), &source), expected);
    }
}

#[test]
fn an_unlabelled_kind_falls_back_to_the_identifier_it_always_was() {
    assert_eq!(
        describe_action(
            &entry(
                "smith:9:card:4",
                json!({"kind": "smith", "card_id": "card:4"})
            ),
            &observation()
        ),
        "smith:9:card:4"
    );
}

#[test]
fn a_malformed_entry_never_panics_and_never_invents() {
    assert_eq!(describe_action(&json!({}), &observation()), "");
    assert_eq!(
        describe_action(&entry("bare:1", json!({})), &observation()),
        "bare:1"
    );
    // A card the observation does not list cannot be named, so the identifier stands.
    assert_eq!(
        describe_action(
            &entry(
                "play:1:card:99:none",
                json!({"kind": "play_card", "card_id": "card:99"})
            ),
            &observation()
        ),
        "play:1:card:99:none"
    );
}

#[test]
fn an_unlisted_target_leaves_the_card_named_without_a_target_clause() {
    assert_eq!(
        describe_action(
            &entry(
                "play:1:card:11:enemy:9",
                json!({"kind": "play_card", "card_id": "card:11", "target_id": "enemy:9"})
            ),
            &observation()
        ),
        "play Strike [1 energy]"
    );
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
