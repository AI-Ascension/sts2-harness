// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use super::seeded_receipt::{SeededAdmission, parse_seeded_admission, validate_first};
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
    pub(super) failure_code: Option<&'static str>,
    pub(super) seeded_admission: Option<SeededAdmission>,
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
        let mut failure_code = None;
        let mut seeded_admission = None;
        for line in text.lines() {
            let row: Value =
                serde_json::from_str(line).map_err(|_| "invalid episode replay JSON")?;
            match row["event"].as_str() {
                Some("seeded_run_receipt") => {
                    if !records.is_empty()
                        || terminal.is_some()
                        || operation.is_some()
                        || seeded_admission.is_some()
                        || failure_code.is_some()
                    {
                        return Err("seeded receipt is not a replay preamble".into());
                    }
                    seeded_admission = Some(parse_seeded_admission(&row)?);
                }
                Some("model_decision") => {
                    if !settled
                        || terminal.is_some()
                        || failure_code.is_some()
                        || records.len() + rejected_attempts >= 1024
                    {
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
                    if settled
                        || terminal.is_some()
                        || failure_code.is_some()
                        || row["action_id"] != record.action_id
                    {
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
                        Some("Accepted")
                            if row["effect"].is_null()
                                && (row["observation"].is_null()
                                    || row["observation"].is_object()) =>
                        {
                            // Admission is not a settlement witness. Keep the action unresolved
                            // until the transition wait or a later reconciliation receipt.
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
                        || failure_code.is_some()
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
                Some("episode_failed") => {
                    if !prefix
                        || terminal.is_some()
                        || !settled
                        || failure_code.is_some()
                        || records.is_empty()
                        || checkpoint.is_none()
                    {
                        return Err("episode replay failure record is invalid".into());
                    }
                    let code = row["error_code"]
                        .as_str()
                        .and_then(failure_code_name)
                        .ok_or("episode replay failure code is invalid")?;
                    if row.as_object().is_none_or(|object| object.len() != 2)
                        || row["event"] != "episode_failed"
                    {
                        return Err("episode replay failure record has an invalid shape".into());
                    }
                    failure_code = Some(code);
                }
                Some("episode_complete") => {
                    if terminal.is_some()
                        || failure_code.is_some()
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
                Some("replay_stream_truncated") => {
                    return Err("episode replay stream was truncated".into());
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
        if let Some(admission) = seeded_admission.as_ref() {
            validate_first(admission, &first.observation, &terminal)?;
        } else {
            let seed = first.observation["visible_seed"]
                .as_str()
                .filter(|seed| !seed.is_empty())
                .ok_or("episode replay requires an explicit visible setup seed")?;
            if first.observation["state"]["state"] != "setup" || terminal["visible_seed"] != seed {
                return Err(
                    "episode replay must span a fresh seeded setup and matching terminal seed"
                        .into(),
                );
            }
        }
        Ok(Self {
            records,
            terminal,
            prefix,
            rejected_attempts,
            failure_code,
            seeded_admission,
        })
    }
}

fn failure_code_name(value: &str) -> Option<&'static str> {
    Some(match value {
        "map_snapshot_invalid" => "map_snapshot_invalid",
        "input_blocked" => "input_blocked",
        "stale_catalog" => "stale_catalog",
        "illegal_action" => "illegal_action",
        "missing_operation" => "missing_operation",
        "malformed_decision" => "malformed_decision",
        "provider_unavailable" => "provider_unavailable",
        "provider_malformed" => "provider_malformed",
        "provider_closed" => "provider_closed",
        "rejected" => "rejected",
        "unknown_outcome" => "unknown_outcome",
        "cleanup" => "cleanup",
        "cleanup_failed" => "cleanup_failed",
        "configuration" => "configuration",
        "configuration_failed" => "configuration_failed",
        "other" => "other",
        _ => return None,
    })
}

pub(super) fn canonical(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("generation");
        object.remove("state_id");
        object.remove("legal_actions");
    }
    // Selection choices identify a catalog, not a pile order. Native grid layout can
    // move a holder between otherwise identical observations. Preserve multiplicity
    // and every choice identity; ordered player piles remain untouched.
    if value["state"]["state"].as_str() == Some("selection")
        && let Some(choices) = value["state"]["choices"].as_array_mut()
        && choices.iter().all(Value::is_string)
    {
        choices.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
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
