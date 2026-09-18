// SPDX-License-Identifier: MIT

//! Exactly derivable combat facts computed from an admitted observation.
//!
//! The fair-play taxonomy in `docs/evidence/runtime-v3-preparation/data/observations.json` declares
//! a `derived_exact` access class: values a calculator may compute from admitted observations, with
//! the note that calculators are not host authority. This module is that calculator, and nothing
//! here reads a host object, a privileged field, or hidden state.
//!
//! It exists because arithmetic is the wrong thing to ask a model for. A provider that is documented
//! to be unreliable at counting, at addition, and at judging numeric proximity should be handed the
//! comparison already made, not the operands. The same projection helps any provider: a model that
//! can add still spends attention doing it.
//!
//! The governing rule is that a value which cannot be derived *exactly* is omitted rather than
//! estimated. An enemy whose intent the host has not revealed does not become an intent of zero
//! damage; it removes the damage total and says so through [`DerivedExactFacts::intents_revealed`].
//!
//! Two limits come from the declared vocabulary in [`super::model_view_fields`] rather than from the
//! game: a card carries `card_id`, `name`, `cost`, and `upgraded` with no attack value, so no
//! lethal claim is derivable here, and neither the player nor an enemy carries block, so incoming
//! damage is reported gross and named to say so.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable schema identity of this projection.
pub const DERIVED_EXACT_SCHEMA: &str = "ascension.context-control.derived-exact.v1";

/// Declared bound on the hand collection, mirroring the model-view vocabulary.
const MAX_HAND: usize = 256;

/// Declared bound on the enemy collection, mirroring the model-view vocabulary.
const MAX_ENEMIES: usize = 64;

/// How a turn's gross incoming damage compares with the player's current hit points.
///
/// The thresholds are policy, not arithmetic, so they are named here rather than left inline: gross
/// incoming damage at or above current hit points is [`Survival::Fatal`], at or above half of them
/// is [`Survival::Heavy`], and anything less is [`Survival::Survivable`]. They describe the gross
/// figure and therefore ignore any mitigation the host applies, which is why a `Fatal` label is a
/// warning rather than a prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Survival {
    /// Gross incoming damage is at least the player's current hit points.
    Fatal,
    /// Gross incoming damage is at least half the player's current hit points.
    Heavy,
    /// Gross incoming damage is below half the player's current hit points.
    Survivable,
}

/// Facts derived exactly from one admitted observation.
///
/// Every optional field is absent when the observation does not support it exactly. Absence is
/// meaningful: it states that no claim is being made, never that the value is zero.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DerivedExactFacts {
    /// Whether every listed enemy carries a revealed intent object.
    pub intents_revealed: bool,
    /// Sum of revealed intent damage multiplied by hits, before any mitigation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incoming_damage_gross: Option<u64>,
    /// How that gross total compares with current hit points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub survival: Option<Survival>,
    /// Hand card identities whose cost is covered by current energy.
    pub affordable_card_ids: Vec<String>,
    /// Hand card identities whose cost is negative and therefore not a fixed number.
    pub variable_cost_card_ids: Vec<String>,
    /// The single lowest-hit-point enemy, absent when two or more share that value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weakest_enemy_id: Option<String>,
    /// Number of cards in hand.
    pub hand_size: usize,
    /// Number of listed enemies.
    pub enemy_count: usize,
}

impl DerivedExactFacts {
    /// Computes the projection from an admitted observation.
    ///
    /// Malformed, absent, or out-of-bound input yields omission rather than a default that would
    /// read as a fact: this constructor never fails and never guesses.
    #[must_use]
    pub fn from_observation(observation: &Value) -> Self {
        let player = observation.get("player");
        let hand = player
            .and_then(|player| player.get("hand"))
            .and_then(Value::as_array);
        let energy = player
            .and_then(|player| player.get("energy"))
            .and_then(Value::as_i64);
        let hit_points = player
            .and_then(|player| player.get("hp"))
            .and_then(Value::as_i64);
        let enemies = observation
            .get("state")
            .and_then(|state| state.get("enemies"))
            .and_then(Value::as_array);

        let (affordable_card_ids, variable_cost_card_ids) = classify_hand(hand, energy);
        let intents_revealed = enemies.is_none_or(|enemies| enemies.iter().all(has_intent));
        let incoming_damage_gross = enemies
            .and_then(|enemies| incoming_damage(enemies))
            .filter(|_| intents_revealed);
        Self {
            intents_revealed,
            survival: incoming_damage_gross.zip(hit_points).map(survival),
            incoming_damage_gross,
            affordable_card_ids,
            variable_cost_card_ids,
            weakest_enemy_id: enemies.and_then(|enemies| weakest_enemy(enemies)),
            hand_size: hand.map_or(0, |hand| hand.len().min(MAX_HAND)),
            enemy_count: enemies.map_or(0, |enemies| enemies.len().min(MAX_ENEMIES)),
        }
    }
}

/// Splits the hand into cards current energy covers and cards whose cost is not a fixed number.
fn classify_hand(hand: Option<&Vec<Value>>, energy: Option<i64>) -> (Vec<String>, Vec<String>) {
    let mut affordable = Vec::new();
    let mut variable = Vec::new();
    for card in hand
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(MAX_HAND)
    {
        let Some(card_id) = card.get("card_id").and_then(Value::as_str) else {
            continue;
        };
        match card.get("cost").and_then(Value::as_i64) {
            Some(cost) if cost < 0 => variable.push(card_id.to_owned()),
            Some(cost) if energy.is_some_and(|energy| cost <= energy) => {
                affordable.push(card_id.to_owned());
            }
            _ => {}
        }
    }
    (affordable, variable)
}

/// Whether the host has revealed this enemy's intent at all.
fn has_intent(enemy: &Value) -> bool {
    enemy.get("intent").is_some_and(Value::is_object)
}

/// Sums revealed intent damage across enemies, refusing a total it cannot state exactly.
///
/// An intent without a `damage` value contributes nothing: its kind is revealed and it is not an
/// attack. An intent whose `damage` or `hits` is present but not a non-negative integer makes the
/// whole total unstatable, because a partial sum would understate the danger.
fn incoming_damage(enemies: &[Value]) -> Option<u64> {
    let mut total: u64 = 0;
    for enemy in enemies.iter().take(MAX_ENEMIES) {
        let intent = enemy.get("intent")?;
        let Some(damage) = intent.get("damage") else {
            continue;
        };
        if damage.is_null() {
            continue;
        }
        let damage = damage.as_u64()?;
        let hits = match intent.get("hits") {
            None => 1,
            Some(hits) if hits.is_null() => 1,
            Some(hits) => hits.as_u64()?,
        };
        total = total.checked_add(damage.checked_mul(hits)?)?;
    }
    Some(total)
}

/// Labels gross incoming damage against current hit points.
fn survival((incoming, hit_points): (u64, i64)) -> Survival {
    let hit_points = hit_points.max(0).unsigned_abs();
    if incoming >= hit_points {
        Survival::Fatal
    } else if incoming.saturating_mul(2) >= hit_points {
        Survival::Heavy
    } else {
        Survival::Survivable
    }
}

/// Names the single lowest-hit-point enemy, or nothing when that value is shared.
fn weakest_enemy(enemies: &[Value]) -> Option<String> {
    let mut weakest: Option<(&str, i64)> = None;
    let mut tied = false;
    for enemy in enemies.iter().take(MAX_ENEMIES) {
        let (Some(enemy_id), Some(hit_points)) = (
            enemy.get("enemy_id").and_then(Value::as_str),
            enemy.get("hp").and_then(Value::as_i64),
        ) else {
            continue;
        };
        match weakest {
            Some((_, lowest)) if hit_points > lowest => {}
            Some((_, lowest)) if hit_points == lowest => tied = true,
            _ => {
                weakest = Some((enemy_id, hit_points));
                tied = false;
            }
        }
    }
    weakest
        .filter(|_| !tied)
        .map(|(enemy_id, _)| enemy_id.to_owned())
}

#[cfg(test)]
#[path = "derived_exact_tests.rs"]
mod tests;
