// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use serde_json::Value;

pub(super) struct ReplayRecord {
    pub(super) action_id: String,
    pub(super) observation: Value,
    pub(super) payload: Value,
}

pub(super) struct ReplayTrace {
    pub(super) records: Vec<ReplayRecord>,
    pub(super) terminal: Value,
    pub(super) prefix: bool,
    pub(super) rejected_attempts: usize,
}

impl ReplayTrace {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, String> {
        Self::parse_mode(bytes, false)
    }

    pub(super) fn parse_mode(bytes: &[u8], prefix: bool) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "episode replay is not UTF-8")?;
        let mut records = Vec::new();
        let mut terminal = None;
        let mut operation: Option<String> = None;
        let mut settled = true;
        let mut checkpoint = None;
        let mut operations = HashSet::new();
        let mut rejected_attempts = 0;
        for line in text.lines() {
            let row: Value =
                serde_json::from_str(line).map_err(|_| "invalid episode replay JSON")?;
            match row["event"].as_str() {
                Some("model_decision") => {
                    if !settled || terminal.is_some() || records.len() + rejected_attempts >= 1024 {
                        return Err("episode replay has unresolved or excess decisions".into());
                    }
                    let id = row["action_id"]
                        .as_str()
                        .filter(|id| !id.is_empty() && id.len() <= 512)
                        .ok_or("invalid recorded action identity")?;
                    let observation = row["observation"].clone();
                    let payload = action_payload(&observation, id)
                        .ok_or("recorded action payload is missing or ambiguous")?
                        .clone();
                    records.push(ReplayRecord {
                        action_id: id.to_owned(),
                        observation,
                        payload,
                    });
                    operation = None;
                    settled = false;
                    checkpoint = None;
                }
                Some("action_receipt") => {
                    let record = records
                        .last()
                        .ok_or("receipt precedes any replay decision")?;
                    if settled || terminal.is_some() || row["action_id"] != record.action_id {
                        return Err("replay receipt does not match its decision".into());
                    }
                    let id = row["operation_id"]
                        .as_str()
                        .filter(|id| !id.is_empty() && id.len() <= 512)
                        .ok_or("invalid replay operation identity")?;
                    if operation.as_ref().is_some_and(|prior| prior != id) {
                        return Err("replay operation identity changed".into());
                    }
                    if operation.is_none() && !operations.insert(id.to_owned()) {
                        return Err("replay operation identity was reused".into());
                    }
                    let first_receipt = operation.is_none();
                    operation = Some(id.to_owned());
                    match row["status"].as_str() {
                        Some("Rejected" | "StaleState")
                            if first_receipt
                                && row["effect"].is_null()
                                && row["observation"].is_object()
                                && record.observation["visible_seed"]
                                    .as_str()
                                    .is_some_and(|seed| !seed.is_empty())
                                && record.observation["visible_seed"]
                                    == row["observation"]["visible_seed"] =>
                        {
                            // A first rejected admission did not dispatch this action. Public
                            // state can advance asynchronously after an earlier settled action.
                            // Preserve its count in replay provenance, but do not dispatch it.
                            records.pop();
                            rejected_attempts += 1;
                            settled = true;
                            operation = None;
                        }
                        Some("Settled")
                            if row["observation"].is_object() && row["effect"].is_string() =>
                        {
                            settled = true;
                            checkpoint = Some(row["observation"].clone());
                        }
                        Some("Unknown") if !settled => {}
                        _ => {
                            return Err(
                                "replay source contains a failed or conflicting receipt".into()
                            );
                        }
                    }
                }
                Some("operation_wait_completed") => {
                    if terminal.is_some()
                        || operation.as_deref() != row["operation_id"].as_str()
                        || operation.is_none()
                        || !row["observation"].is_object()
                        || !row["effect"].is_string()
                    {
                        return Err("replay settlement does not match its operation".into());
                    }
                    if settled
                        && checkpoint.as_ref().map(canonical)
                            != Some(canonical(&row["observation"]))
                    {
                        return Err("replay settlement observation changed".into());
                    }
                    settled = true;
                    checkpoint = Some(row["observation"].clone());
                }
                Some("episode_complete") => {
                    if terminal.is_some()
                        || !settled
                        || records.is_empty()
                        || !matches!(
                            row["observation"]["state"]["state"].as_str(),
                            Some("victory" | "defeat")
                        )
                    {
                        return Err("episode replay terminal record is invalid".into());
                    }
                    if checkpoint.as_ref().map(canonical) != Some(canonical(&row["observation"])) {
                        return Err(
                            "replay terminal does not match final settled observation".into()
                        );
                    }
                    terminal = Some(row["observation"].clone());
                }
                Some(_) => return Err("unsupported episode replay record".into()),
                None if row["protocol"] == "runtime-v3-gameplay"
                    && row["status"] == "complete"
                    && terminal.is_some() => {}
                None => return Err("episode replay record omitted its event".into()),
            }
        }
        let terminal = if prefix {
            if terminal.is_some() || !settled {
                return Err("prefix replay requires a settled nonterminal checkpoint".into());
            }
            let checkpoint = checkpoint.ok_or("prefix replay has no settled checkpoint")?;
            if !checkpoint["legal_actions"]
                .as_array()
                .is_some_and(|actions| !actions.is_empty())
                || matches!(
                    checkpoint["state"]["state"].as_str(),
                    Some("victory" | "defeat" | "recovery")
                )
            {
                return Err("prefix replay checkpoint is not actionable".into());
            }
            checkpoint
        } else {
            terminal.ok_or("episode replay is incomplete")?
        };
        let first = records.first().ok_or("episode replay has no decisions")?;
        let seed = first.observation["visible_seed"]
            .as_str()
            .filter(|seed| !seed.is_empty())
            .ok_or("episode replay requires an explicit visible setup seed")?;
        if first.observation["state"]["state"] != "setup" || terminal["visible_seed"] != seed {
            return Err(
                "episode replay must span a fresh seeded setup and matching terminal seed".into(),
            );
        }
        Ok(Self {
            records,
            terminal,
            prefix,
            rejected_attempts,
        })
    }
}

pub(super) fn canonical(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("generation");
        object.remove("state_id");
        object.remove("legal_actions");
    }
    value
}

pub(super) fn action_payload<'a>(observation: &'a Value, id: &str) -> Option<&'a Value> {
    let mut matches = observation["legal_actions"]
        .as_array()?
        .iter()
        .filter(|entry| entry["action_id"].as_str() == Some(id));
    let payload = matches.next()?.get("action")?;
    (matches.next().is_none() && payload.is_object()).then_some(payload)
}
