// SPDX-License-Identifier: MIT

//! Composes an option description from the admitted observation.
//!
//! A System One choice question is a map of option identifier to description, and presenting the
//! identifier as its own description asks the model to resolve `play:13:card:12:none` against the
//! hand itself. That is a join, and splitting probability mass across strings that differ only in a
//! card ordinal is precisely the distractor sensitivity the model is documented to have.
//!
//! Nothing here describes what an action *does*. Every word is one of two things: a value the host
//! supplied in this same observation, or a fixed label for the host's own action `kind`. There is no
//! table of card effects, no damage number, and no advice, because this module has no source for any
//! of that and a second, unverified account of the game in front of the model is worse than a join.
//!
//! A field the observation does not carry is omitted rather than defaulted, so a description is
//! always a subset of what the host said. When an action kind is not one this module labels, the
//! identifier is returned unchanged, which is exactly the previous behaviour.

use serde_json::Value;

/// Largest description this module emits, matching the option bound in the request builder.
const MAX_DESCRIPTION_BYTES: usize = 240;

/// Describes one legal-action entry using only values from this observation.
///
/// `entry` is a `legal_actions` element carrying `action_id` and `action`. Returns the identifier
/// unchanged when the entry is malformed or its kind is not labelled here.
#[must_use]
pub fn describe_action(entry: &Value, observation: &Value) -> String {
    let Some(action_id) = entry.get("action_id").and_then(Value::as_str) else {
        return String::new();
    };
    let described = entry
        .get("action")
        .and_then(|action| compose(action, observation));
    match described {
        Some(text) if text.len() <= MAX_DESCRIPTION_BYTES => text,
        _ => action_id.to_owned(),
    }
}

/// Composes the description for one action object, or nothing for an unlabelled kind.
fn compose(action: &Value, observation: &Value) -> Option<String> {
    let kind = action.get("kind").and_then(Value::as_str)?;
    let text = match kind {
        "play_card" => {
            let card = card_phrase(action.get("card_id").and_then(Value::as_str), observation)?;
            match target_phrase(action.get("target_id").and_then(Value::as_str), observation) {
                Some(target) => format!("play {card} at {target}"),
                None => format!("play {card}"),
            }
        }
        "end_turn" => String::from("end the turn"),
        "start_run" => format!("start a run as {}", named(action, "character_id")?),
        "select_map_node" => format!("travel to map node {}", named(action, "node_id")?),
        "choose_reward" => {
            let reward_id = named(action, "reward_id")?;
            format!(
                "take the reward {}",
                offered_phrase(&reward_id, observation).unwrap_or(reward_id)
            )
        }
        "select_card" => {
            let card_id = named(action, "card_id").unwrap_or_default();
            let phrase = card_phrase(Some(&card_id), observation)
                .or_else(|| offered_phrase(&card_id, observation))
                .unwrap_or(card_id);
            format!("choose {phrase}")
        }
        "use_potion" => {
            let potion =
                potion_phrase(action.get("potion_id").and_then(Value::as_str), observation)?;
            match target_phrase(action.get("target_id").and_then(Value::as_str), observation) {
                Some(target) => format!("use {potion} at {target}"),
                None => format!("use {potion}"),
            }
        }
        "discard_potion" => format!(
            "discard {}",
            potion_phrase(action.get("potion_id").and_then(Value::as_str), observation)?
        ),
        "skip_reward" => skip_phrase(observation),
        "proceed" => String::from("proceed"),
        "shop_purchase" => format!("buy {}", item_phrase(action, observation)?),
        "shop_remove" => format!(
            "remove {} from the deck",
            card_phrase(action.get("card_id").and_then(Value::as_str), observation)
                .unwrap_or_else(|| named(action, "card_id").unwrap_or_default())
        ),
        "event_choice" => format!("choose the option {}", named(action, "choice_id")?),
        "confirm_selection" => String::from("confirm the selection"),
        "cancel_selection" => String::from("cancel the selection"),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

/// Reads one non-empty string field from the action object.
fn named(action: &Value, field: &str) -> Option<String> {
    action
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Names a card from the hand, falling back to the deck and discard for a card not held.
///
/// The cost is stated only when the host gives a non-negative number: a negative cost is the host's
/// way of saying the cost is not fixed, and printing it as a number would be a claim it did not
/// make. `description` is included when the host carries one, and is the host's own card text.
fn card_phrase(card_id: Option<&str>, observation: &Value) -> Option<String> {
    let card_id = card_id?;
    let card = find_card(card_id, observation)?;
    let name = card.get("name").and_then(Value::as_str).unwrap_or(card_id);
    let mut phrase = String::from(name);
    if card.get("upgraded").and_then(Value::as_bool) == Some(true) {
        phrase.push_str(" (upgraded)");
    }
    if let Some(cost) = card.get("cost").and_then(Value::as_i64).filter(|c| *c >= 0) {
        phrase.push_str(&format!(" [{cost} energy]"));
    }
    if let Some(text) = card
        .get("description")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        phrase.push_str(&format!(": {text}"));
    }
    Some(phrase)
}

/// Names a potion the player is carrying, with the host's own text for what it does.
///
/// A potion is an action available this turn rather than a passive holding, so an unnamed one is a
/// legal action the model cannot tell apart from any other. A potion the observation does not list
/// yields nothing, and the identifier stands.
fn potion_phrase(potion_id: Option<&str>, observation: &Value) -> Option<String> {
    let potion_id = potion_id?;
    let potion = observation
        .get("player")?
        .get("potions")?
        .as_array()?
        .iter()
        .find(|potion| potion.get("potion_id").and_then(Value::as_str) == Some(potion_id))?;
    let name = potion
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(potion_id);
    let mut phrase = String::from(name);
    if let Some(text) = potion
        .get("description")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        phrase.push_str(&format!(": {text}"));
    }
    Some(phrase)
}

/// Describes declining an offer, naming what is being declined.
///
/// "skip the reward" reads as a tidy, legible option next to a list of identifiers the model cannot
/// rank, and it won a recorded reward loop repeatedly at low confidence. Naming what is given up
/// puts the trade in front of the model rather than leaving skipping as the one option that needs
/// no reading.
///
/// It states the count, which the host gave, and nothing else. It does not say the offered cards
/// are unsuitable, or that skipping is wise: neither is known here, and a description that argued
/// for one option would be choosing instead of describing.
fn skip_phrase(observation: &Value) -> String {
    let offered = observation
        .get("state")
        .and_then(|state| {
            ["choices", "options"]
                .into_iter()
                .find_map(|key| state.get(key))
        })
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    match offered {
        0 => String::from("skip the reward, taking nothing from it"),
        count => format!("skip the reward, taking none of the {count} offered"),
    }
}

/// Describes an offered card or reward from the set the host listed for this screen.
///
/// An offered entry is not held in any pile, so it cannot be found by [`find_card`]. A host that
/// lists the set as bare identifiers gives nothing to add, and this returns nothing; a host that
/// describes it is quoted. The offered set is unmodeled upstream today, so the second case is
/// the capacity rather than the current behaviour.
fn offered_phrase(id: &str, observation: &Value) -> Option<String> {
    let state = observation.get("state")?;
    let entry = ["choices", "options"]
        .into_iter()
        .filter_map(|key| state.get(key).and_then(Value::as_array))
        .flatten()
        .find(|entry| entry.get("choice_id").and_then(Value::as_str) == Some(id))?;
    let mut phrase = describe_entry(entry);
    if let Some(contents) = disclosed_contents(entry) {
        phrase.push_str(&format!(", offering {contents}"));
    }
    Some(phrase)
}

/// Lists what an option would present next, when the host discloses it.
///
/// Taking a card reward opens a second screen holding the cards. Described here, the choice of
/// whether to open it at all is made knowing what is inside, rather than after the fact. An entry
/// the host lists as a bare identifier contributes that identifier, because that is all it said.
fn disclosed_contents(entry: &Value) -> Option<String> {
    let contents = entry.get("contents")?.as_array()?;
    let described: Vec<String> = contents
        .iter()
        .filter_map(|item| match item {
            Value::String(id) => Some(id.clone()),
            Value::Object(_) => Some(describe_entry(item)),
            _ => None,
        })
        .collect();
    (!described.is_empty()).then(|| described.join("; "))
}

/// Names one described entry: the shared rendering of a card, a reward, or a disclosed content.
fn describe_entry(entry: &Value) -> String {
    let id = entry
        .get("choice_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = entry.get("name").and_then(Value::as_str).unwrap_or(id);
    let mut phrase = String::from(name);
    if entry.get("upgraded").and_then(Value::as_bool) == Some(true) {
        phrase.push_str(" (upgraded)");
    }
    if let Some(cost) = entry
        .get("cost")
        .and_then(Value::as_i64)
        .filter(|c| *c >= 0)
    {
        phrase.push_str(&format!(" [{cost} energy]"));
    }
    if let Some(rarity) = entry
        .get("rarity")
        .and_then(Value::as_str)
        .filter(|rarity| !rarity.is_empty())
    {
        phrase.push_str(&format!(" ({rarity})"));
    }
    if let Some(text) = entry
        .get("description")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        phrase.push_str(&format!(": {text}"));
    }
    phrase
}

/// Finds a card object by identity in any collection the observation lists it in.
fn find_card<'a>(card_id: &str, observation: &'a Value) -> Option<&'a Value> {
    let player = observation.get("player")?;
    ["hand", "deck", "discard", "exhaust"]
        .into_iter()
        .filter_map(|pile| player.get(pile).and_then(Value::as_array))
        .flatten()
        .find(|card| card.get("card_id").and_then(Value::as_str) == Some(card_id))
}

/// Names an enemy target with its remaining hit points, or nothing for an unlisted target.
fn target_phrase(target_id: Option<&str>, observation: &Value) -> Option<String> {
    let target_id = target_id?;
    let enemy = observation
        .get("state")?
        .get("enemies")?
        .as_array()?
        .iter()
        .find(|enemy| enemy.get("enemy_id").and_then(Value::as_str) == Some(target_id))?;
    let name = enemy
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(target_id);
    match enemy.get("hp").and_then(Value::as_i64) {
        Some(hp) => Some(format!("{name} ({hp} hit points left)")),
        None => Some(String::from(name)),
    }
}

/// Names a shop item with its price, from the item list in the observation.
fn item_phrase(action: &Value, observation: &Value) -> Option<String> {
    let item_id = action.get("item_id").and_then(Value::as_str)?;
    let item = observation
        .get("state")?
        .get("items")?
        .as_array()?
        .iter()
        .find(|item| item.get("item_id").and_then(Value::as_str) == Some(item_id))?;
    let name = item.get("name").and_then(Value::as_str).unwrap_or(item_id);
    match item.get("price").and_then(Value::as_i64) {
        Some(price) => Some(format!("{name} for {price} gold")),
        None => Some(String::from(name)),
    }
}

#[cfg(test)]
#[path = "action_description_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "action_description_offer_tests.rs"]
mod offer_tests;
