// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest, Sha256};
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
            .cloned()
            .ok_or("replay trajectory is incomplete")?;
        let has_digest = terminal["terminal_observation_digest"]
            .as_str()
            .is_some_and(valid_digest);
        let has_legacy_observation = terminal["observation"].is_object();
        if !has_digest && !has_legacy_observation {
            return Err("missing terminal replay observation digest".into());
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

    pub(super) fn observation_digest(observation: &EpisodeObservation) -> String {
        digest_value(
            "combat-observation",
            &canonical(observation.fair_play().as_value().clone()),
        )
    }

    pub(super) fn action_digest(action: &Value) -> String {
        digest_value("combat-action", action)
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
        let matching: Vec<_> = if let (Some(expected), Some(recorded)) = (
            record["observation_digest"].as_str(),
            record["action_digest"].as_str(),
        ) {
            if expected != Self::observation_digest(before) {
                return Err(format!(
                    "replay observation diverged before step {}",
                    step + 1
                ));
            }
            actions
                .actions()
                .iter()
                .filter(|action| {
                    action_payload(before.fair_play().as_value(), action.action_id())
                        .is_some_and(|payload| Self::action_digest(payload) == recorded)
                })
                .collect()
        } else {
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
            actions
                .actions()
                .iter()
                .filter(|a| {
                    action_payload(before.fair_play().as_value(), a.action_id()) == Some(key)
                })
                .collect()
        };
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
        if self
            .terminal
            .as_ref()
            .is_some_and(|expected| !terminal_matches(expected, observation))
        {
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

fn digest_value(domain: &str, value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(canonical_json(value));
    sts2_harness::hex_bytes(hasher.finalize())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn terminal_matches(expected: &Value, observation: &EpisodeObservation) -> bool {
    match expected.get("terminal_observation_digest") {
        Some(Value::String(digest)) => digest == &Replay::observation_digest(observation),
        Some(Value::Null) | None => {
            expected["observation"].is_object()
                && canonical(expected["observation"].clone())
                    == canonical(observation.fair_play().as_value().clone())
        }
        Some(_) => false,
    }
}

fn canonical_json(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => b"null".to_vec(),
        Value::Bool(value) => value.to_string().into_bytes(),
        Value::Number(value) => value.to_string().into_bytes(),
        Value::String(value) => serde_json::to_vec(value).unwrap_or_default(),
        Value::Array(values) => {
            let mut bytes = vec![b'['];
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                bytes.extend(canonical_json(value));
            }
            bytes.push(b']');
            bytes
        }
        Value::Object(values) => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let mut bytes = vec![b'{'];
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                bytes.extend(serde_json::to_vec(key).unwrap_or_default());
                bytes.push(b':');
                bytes.extend(canonical_json(value));
            }
            bytes.push(b'}');
            bytes
        }
    }
}

pub(super) fn action_payload<'a>(observation: &'a Value, id: &str) -> Option<&'a Value> {
    let mut matches = observation["legal_actions"]
        .as_array()?
        .iter()
        .filter(|entry| entry["action_id"].as_str() == Some(id));
    let payload = matches.next()?.get("action")?;
    (matches.next().is_none() && payload.is_object()).then_some(payload)
}

#[cfg(test)]
include!("runtime_v3_combat_replay_tests.rs");
