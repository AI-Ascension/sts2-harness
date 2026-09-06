// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::Value;

use super::trace::canonical;

#[derive(Clone, Default)]
pub(super) struct CardBindings(BTreeMap<String, String>);

impl CardBindings {
    pub(super) fn reconcile(&self, recorded: &Value, current: &Value) -> Option<Self> {
        let mut next = self.clone();
        for pile in ["deck", "hand", "discard", "exhaust"] {
            let left = recorded["player"][pile].as_array()?;
            let right = current["player"][pile].as_array()?;
            if left.len() != right.len() {
                return None;
            }
            for (source, target) in left.iter().zip(right) {
                next.bind(source, target, left)?;
            }
        }
        (canonical(&next.translate(recorded)) == canonical(current)).then_some(next)
    }

    fn bind(&mut self, source: &Value, target: &Value, pile: &[Value]) -> Option<()> {
        let from = source["card_id"].as_str()?;
        let to = target["card_id"].as_str()?;
        if card_content(source)? != card_content(target)? {
            return None;
        }
        if let Some(prior) = self.0.get(from) {
            return (prior == to).then_some(());
        }
        if self.0.values().any(|bound| bound == to) {
            return None;
        }
        // A new changed identity needs a unique public card in this ordered pile.
        // Previously established bindings remain authoritative for duplicate cards.
        if from != to
            && pile
                .iter()
                .filter(|card| card_content(card) == card_content(source))
                .count()
                != 1
        {
            return None;
        }
        self.0.insert(from.to_owned(), to.to_owned());
        Some(())
    }

    pub(super) fn translate(&self, value: &Value) -> Value {
        match value {
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| {
                        let translated = if key == "card_id" {
                            self.identity(value)
                        } else if key == "choices" {
                            value
                                .as_array()
                                .map(|choices| {
                                    Value::Array(
                                        choices
                                            .iter()
                                            .map(|choice| self.identity(choice))
                                            .collect(),
                                    )
                                })
                                .unwrap_or_else(|| value.clone())
                        } else {
                            self.translate(value)
                        };
                        (key.clone(), translated)
                    })
                    .collect(),
            ),
            Value::Array(values) => {
                Value::Array(values.iter().map(|v| self.translate(v)).collect())
            }
            _ => value.clone(),
        }
    }

    fn identity(&self, value: &Value) -> Value {
        value
            .as_str()
            .and_then(|id| self.0.get(id))
            .map(|id| Value::String(id.clone()))
            .unwrap_or_else(|| value.clone())
    }
}

fn card_content(value: &Value) -> Option<Value> {
    let mut object = value.as_object()?.clone();
    object.remove("card_id")?;
    Some(Value::Object(object))
}

#[cfg(test)]
#[path = "runtime_v3_replay_cards_tests.rs"]
mod tests;
