// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::io::Read;
use std::path::Path;
use sts2_harness::{Decision, EpisodeLegalActionSet, EpisodeObservation};

pub(super) struct Replay {
    records: Option<Vec<Value>>,
    terminal: Option<Value>,
}

impl Replay {
    pub(super) fn load(path: Option<&Path>) -> Result<Self, String> {
        let Some(path) = path else {
            return Ok(Self {
                records: None,
                terminal: None,
            });
        };
        let file = std::fs::File::open(path).map_err(|_| "cannot open replay trajectory")?;
        let mut text = String::new();
        file.take(8 * 1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|_| "cannot read replay trajectory")?;
        if text.len() > 8 * 1024 * 1024 {
            return Err("replay trajectory exceeds bound".into());
        }
        let values = text
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "invalid replay JSON")?;
        let terminal = values
            .last()
            .filter(|v| v["event"] == "combat_demo_complete")
            .ok_or("replay trajectory is incomplete")?["observation"]
            .clone();
        if !terminal.is_object() {
            return Err("missing terminal replay observation".into());
        }
        let records: Vec<_> = values
            .into_iter()
            .filter(|v| v["event"] == "model_decision")
            .collect();
        if records.is_empty() || records.len() > 100 {
            return Err("invalid replay action count".into());
        }
        Ok(Self {
            records: Some(records),
            terminal: Some(terminal),
        })
    }

    pub(super) fn event(&self) -> &'static str {
        if self.records.is_some() {
            "replay_decision"
        } else {
            "model_decision"
        }
    }

    pub(super) fn decide(
        &self,
        step: u32,
        before: &EpisodeObservation,
        actions: &EpisodeLegalActionSet,
    ) -> Result<Option<Decision>, String> {
        let Some(records) = &self.records else {
            return Ok(None);
        };
        let record = records
            .get(step as usize)
            .ok_or("replay action sequence exhausted")?;
        if canonical(record["observation"].clone())
            != canonical(before.fair_play().as_value().clone())
        {
            return Err(format!(
                "replay observation diverged before step {}",
                step + 1
            ));
        }
        let recorded = record["action_id"]
            .as_str()
            .ok_or("missing replay action")?;
        let key = action_payload(&record["observation"], recorded)
            .ok_or("recorded action payload is missing or ambiguous")?;
        let matching: Vec<_> = actions
            .actions()
            .iter()
            .filter(|a| action_payload(before.fair_play().as_value(), a.action_id()) == Some(key))
            .collect();
        if matching.len() != 1 {
            return Err("replay action is not uniquely legal".into());
        }
        Ok(Some(Decision::Action {
            action_id: matching[0].action_id().to_owned(),
            rationale: "Replay of recorded model action; no inference".into(),
            confidence: None,
        }))
    }

    pub(super) fn finish(
        &self,
        steps: u32,
        observation: &EpisodeObservation,
    ) -> Result<(), String> {
        if self
            .records
            .as_ref()
            .is_some_and(|r| r.len() != steps as usize)
        {
            return Err("combat ended before replay sequence completed".into());
        }
        if self.terminal.as_ref().is_some_and(|expected| {
            canonical(expected.clone()) != canonical(observation.fair_play().as_value().clone())
        }) {
            return Err("terminal replay observation diverged".into());
        }
        Ok(())
    }
}

fn canonical(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        // These identify this live observation, not seeded game content. The selected
        // semantic action is separately checked against the fresh authoritative catalog.
        object.remove("generation");
        object.remove("state_id");
        object.remove("legal_actions");
    }
    value
}

fn action_payload<'a>(observation: &'a Value, id: &str) -> Option<&'a Value> {
    let mut matches = observation["legal_actions"]
        .as_array()?
        .iter()
        .filter(|entry| entry["action_id"].as_str() == Some(id));
    let payload = matches.next()?.get("action")?;
    (matches.next().is_none() && payload.is_object()).then_some(payload)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn shared_model_execution_replays_each_step_with_its_fresh_semantic_payload() {
        use sts2_harness::{ActionKind, EpisodeLegalAction, EpisodeStage};
        let make = |generation: u64, card: &str, id: &str| {
            json!({
                "state_id":format!("combat-{generation}"),"generation":generation,"visible_seed":"seed",
                "player":{"hp":50,"max_hp":50,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
                "state":{"state":"combat","turn_index":1,"enemies":[]},
                "legal_actions":[{"action_id":id,"action":{"kind":"play_card","card_id":card,"target_id":null}}]
            })
        };
        let records = ["a", "b"].iter().enumerate().map(|(index, card)| {
            let id = format!("recorded-{index}");
            json!({"event":"model_decision","model_execution_id":7,
                "reused_model_execution":index > 0,"action_id":id,"observation":make(index as u64,card,&id)})
        }).collect();
        let replay = Replay {
            records: Some(records),
            terminal: None,
        };
        for (index, card) in ["a", "b"].iter().enumerate() {
            let generation = index as u64 + 20;
            let id = format!("fresh-{index}");
            let observation = EpisodeObservation::new(
                format!("combat-{generation}"),
                generation,
                EpisodeStage::Combat,
                true,
                false,
                true,
                make(generation, card, &id),
            )
            .expect("fixture observation");
            let actions = EpisodeLegalActionSet::new(
                observation.state_id(),
                generation,
                vec![EpisodeLegalAction::new(&id, ActionKind::PlayCard).expect("fixture action")],
            )
            .expect("fixture catalog");
            assert!(matches!(replay.decide(index as u32,&observation,&actions),
                Ok(Some(Decision::Action {action_id,..})) if action_id == id));
            let wrong = EpisodeObservation::new(
                format!("combat-{generation}"),
                generation,
                EpisodeStage::Combat,
                true,
                false,
                true,
                make(generation, "wrong-target-card", &id),
            )
            .expect("fixture observation");
            assert!(replay.decide(index as u32, &wrong, &actions).is_err());
        }
    }

    #[test]
    fn replay_normalizes_only_observation_identity_and_catalog() {
        let first = json!({"legal_actions":[{"action_id":"old-id", "action":{"kind":"end_turn"}}]});
        let next =
            json!({"legal_actions":[{"action_id":"fresh-id", "action":{"kind":"end_turn"}}]});
        assert_eq!(
            action_payload(&first, "old-id"),
            action_payload(&next, "fresh-id")
        );
        assert!(action_payload(&next, "old-id").is_none());
        assert_ne!(
            canonical(json!({"visible_seed":"A","generation":1})),
            canonical(json!({"visible_seed":"B","generation":2}))
        );
        assert_ne!(
            canonical(json!({"player":{"hp":1}})),
            canonical(json!({"player":{"hp":2}}))
        );
    }
}
