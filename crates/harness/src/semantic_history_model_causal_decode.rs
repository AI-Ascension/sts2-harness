// SPDX-License-Identifier: MIT

//! Decoding one causal parent, where the two arms must never be mixed.

use super::SemanticHistoryCausalParent;
use serde::Deserialize;

/// The encoded shape of a causal parent: a stated cause names exactly one event, and an unstated one
/// names nothing at all.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct Wire {
    state: State,
    event_id: Option<String>,
}

/// The two admitted arms.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum State {
    /// A cause was stated.
    Stated,
    /// No cause was stated.
    NotStated,
}

impl<'de> Deserialize<'de> for SemanticHistoryCausalParent {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let wire = Wire::deserialize(decoder)?;
        match (wire.state, wire.event_id) {
            (State::Stated, Some(event_id)) if !event_id.is_empty() => {
                Ok(Self::Stated { event_id })
            }
            (State::NotStated, None) => Ok(Self::NotStated),
            // A pairing that mixes the arms is refused rather than read as one of them, so
            // "we do not know why" can never decode as "this was the cause".
            _ => Err(<D::Error as serde::de::Error>::custom(
                "semantic history: causal parent mixes its arms",
            )),
        }
    }
}
