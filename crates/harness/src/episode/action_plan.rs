// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

use serde_json::Value;

use super::policy_router::{DecisionInput, PolicyError};
use crate::exo::Decision;
use crate::identity::ModelExecutionId;

/// An ordered provider plan; host legality and settlement remain authoritative.
pub(super) struct ActionPlan {
    actions: VecDeque<Value>,
    expected: Value,
    rationale: String,
    pub(super) execution_id: ModelExecutionId,
    settled: bool,
    generation: u64,
    objective: String,
    constraints: Vec<String>,
}

impl ActionPlan {
    pub(super) fn awaiting_settlement(&self) -> bool {
        !self.settled
    }

    pub(super) fn new(
        input: &DecisionInput,
        ids: &[String],
        rationale: String,
    ) -> Result<Self, PolicyError> {
        if ids.is_empty() || ids.len() > 8 {
            return Err(PolicyError::MalformedDecision);
        }
        let mut actions = VecDeque::new();
        for (index, id) in ids.iter().enumerate() {
            if ids[..index].contains(id) || input.legal_actions.find(id).is_none() {
                return Err(PolicyError::IllegalAction);
            }
            actions.push_back(
                payload(input, id)
                    .ok_or(PolicyError::MalformedDecision)?
                    .clone(),
            );
        }
        if ids.len() > 1 && !permitted_sequence(input, &actions) {
            return Err(PolicyError::MalformedDecision);
        }
        Ok(Self {
            actions,
            expected: input.observation.fair_play().as_value().clone(),
            rationale,
            execution_id: input.execution_id,
            settled: true,
            generation: input.observation.generation(),
            objective: input.objective.clone(),
            constraints: input.hard_constraints.clone(),
        })
    }

    pub(super) fn action_completed(&mut self, settled: bool) {
        self.settled = settled;
        if !settled {
            self.actions.clear();
        }
    }

    pub(super) fn next(&mut self, input: &DecisionInput, initial: bool) -> Option<Decision> {
        if !self.settled || (!initial && !self.compatible(input)) {
            return None;
        }
        let action = self.actions.front()?;
        let mut candidates = input
            .legal_actions
            .actions()
            .iter()
            .filter(|candidate| payload(input, candidate.action_id()) == Some(action));
        let id = candidates.next()?.action_id().to_owned();
        if candidates.next().is_some() {
            return None;
        }
        self.expected = input.observation.fair_play().as_value().clone();
        remove_consumed(&mut self.expected, action);
        self.actions.pop_front();
        self.generation = input.observation.generation();
        self.settled = false;
        Some(Decision::Action {
            action_id: id,
            rationale: self.rationale.clone(),
            confidence: None,
        })
    }

    fn compatible(&self, input: &DecisionInput) -> bool {
        let current = input.observation.fair_play().as_value();
        if input.observation.generation() <= self.generation
            || input.objective != self.objective
            || input.hard_constraints != self.constraints
            || current["visible_seed"] != self.expected["visible_seed"]
            || current["state"]["state"] != self.expected["state"]["state"]
            || current["player"]["hp"] != self.expected["player"]["hp"]
            || current["player"]["max_hp"] != self.expected["player"]["max_hp"]
        {
            return false;
        }
        match current["state"]["state"].as_str() {
            Some("combat") => {
                current["state"]["turn_index"] == self.expected["state"]["turn_index"]
                    && subset(&current["player"]["hand"], &self.expected["player"]["hand"])
                    && same_enemy_intents(
                        &current["state"]["enemies"],
                        &self.expected["state"]["enemies"],
                    )
            }
            Some("shop") => subset(&current["state"]["items"], &self.expected["state"]["items"]),
            _ => false,
        }
    }
}

fn payload<'a>(input: &'a DecisionInput, id: &str) -> Option<&'a Value> {
    let catalog = input.observation.fair_play().as_value()["legal_actions"].as_array()?;
    let mut matches = catalog
        .iter()
        .filter(|item| item["action_id"].as_str() == Some(id));
    let action = matches.next()?.get("action")?;
    (matches.next().is_none() && action.is_object()).then_some(action)
}

fn permitted_sequence(input: &DecisionInput, actions: &VecDeque<Value>) -> bool {
    let stage = input.observation.fair_play().as_value()["state"]["state"].as_str();
    actions.iter().enumerate().all(|(index, action)| {
        let last = index + 1 == actions.len();
        matches!(
            (stage, action["kind"].as_str()),
            (Some("combat"), Some("play_card")) | (Some("shop"), Some("shop_purchase"))
        ) || last
            && matches!(
                (stage, action["kind"].as_str()),
                (Some("combat"), Some("end_turn")) | (Some("shop"), Some("proceed"))
            )
    })
}

fn remove_consumed(expected: &mut Value, action: &Value) {
    let (items, id_key, selected) = match action["kind"].as_str() {
        Some("play_card") => (
            &mut expected["player"]["hand"],
            "card_id",
            &action["card_id"],
        ),
        Some("shop_purchase") => (
            &mut expected["state"]["items"],
            "item_id",
            &action["item_id"],
        ),
        _ => return,
    };
    if let Some(items) = items.as_array_mut() {
        items.retain(|item| &item[id_key] != selected);
    }
}

fn subset(current: &Value, previous: &Value) -> bool {
    let (Some(current), Some(previous)) = (current.as_array(), previous.as_array()) else {
        return false;
    };
    let mut remaining = previous.clone();
    current.iter().all(|item| {
        if let Some(index) = remaining.iter().position(|candidate| candidate == item) {
            remaining.remove(index);
            true
        } else {
            false
        }
    })
}

fn same_enemy_intents(current: &Value, previous: &Value) -> bool {
    let (Some(current), Some(previous)) = (current.as_array(), previous.as_array()) else {
        return false;
    };
    current.iter().all(|enemy| {
        previous.iter().any(|old| {
            enemy["enemy_id"] == old["enemy_id"]
                && enemy["name"] == old["name"]
                && enemy["intent"] == old["intent"]
                && enemy["max_hp"] == old["max_hp"]
        })
    })
}
