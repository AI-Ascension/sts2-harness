// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::io::Read;
use sts2_harness::{Decision, EpisodeLegalActionSet, EpisodeObservation};

pub(super) struct Replay {
    records: Option<Vec<Value>>,
    terminal: Option<Value>,
}

impl Replay {
    pub(super) fn load() -> Result<Self, String> {
        let path = std::env::var("STS2_REPLAY_TRAJECTORY").unwrap_or_default();
        if path.is_empty() {
            return Ok(Self {
                records: None,
                terminal: None,
            });
        }
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
        let key = semantic_action(recorded)?;
        let matching: Vec<_> = actions
            .actions()
            .iter()
            .filter(|a| semantic_action(a.action_id()).ok().as_deref() == Some(key.as_str()))
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

fn semantic_action(id: &str) -> Result<String, String> {
    let parts: Vec<_> = id.splitn(3, ':').collect();
    match parts.as_slice() {
        ["end", generation] if generation.parse::<u64>().is_ok() => Ok("end".into()),
        ["play", generation, rest] if generation.parse::<u64>().is_ok() => {
            Ok(format!("play:{rest}"))
        }
        _ => Err("unsupported replay action identity".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn replay_normalizes_only_observation_identity_and_catalog() {
        assert_eq!(
            semantic_action("play:3:card:1:enemy:2"),
            semantic_action("play:8:card:1:enemy:2")
        );
        assert_ne!(
            semantic_action("play:3:card:1:enemy:2"),
            semantic_action("play:3:card:1:enemy:1")
        );
        assert!(semantic_action("play:x:card:1:none").is_err());
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
