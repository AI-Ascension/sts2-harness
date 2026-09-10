// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::{HandoffError, PAYLOAD_DIGEST, SCHEMA_DIGEST, json};

pub(super) const HEADER: &[&str] = &[
    "contract",
    "schema_digest",
    "direction",
    "command",
    "scope",
    "request_id",
    "timeout_ms",
    "watchdog_boot_id",
];
pub(super) const TUPLE: &[&str] = &[
    "handoff_id",
    "deployment_id",
    "job_id",
    "attempt_id",
    "attempt_number",
    "worker_owner_id",
    "worker_profile_digest",
    "run_id",
    "episode_id",
    "trajectory_id",
    "payload_digest",
];
pub(super) const CONTROL: &[&str] = &[
    "deployment_id",
    "worker_owner_id",
    "worker_profile_digest",
    "mode",
    "mode_sequence",
];

/// Command determines the required authorization capability at the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerCommand {
    Probe,
    Dispatch,
    Lookup,
    Acknowledge,
    SetControlMode,
}

/// Immutable, closed and bounded request. Authentication and durable admission
/// must occur separately before a command can have effects.
#[derive(Clone)]
pub struct WorkerRequest {
    command: WorkerCommand,
    fields: Map<String, Value>,
}

impl WorkerRequest {
    /// Reject malformed JSON, duplicate/unknown fields, noncanonical numbers,
    /// incompatible schemas, malformed identities and unsupported operations.
    pub fn decode(bytes: &[u8]) -> Result<Self, HandoffError> {
        let Value::Object(fields) = json::decode(bytes)? else {
            return Err(HandoffError);
        };
        constant(&fields, "contract", "ascension-watchdog-worker-handoff-v1")?;
        constant(&fields, "schema_digest", SCHEMA_DIGEST)?;
        constant(&fields, "direction", "request")?;
        uuid(&fields, "request_id")?;
        uuid(&fields, "watchdog_boot_id")?;
        positive(&fields, "timeout_ms")?;
        if fields["timeout_ms"].as_u64().ok_or(HandoffError)? > 5000 {
            return Err(HandoffError);
        }
        let command = match string(&fields, "command")? {
            "probe" => WorkerCommand::Probe,
            "dispatch" => WorkerCommand::Dispatch,
            "lookup" => WorkerCommand::Lookup,
            "acknowledge" => WorkerCommand::Acknowledge,
            "set_control_mode" => WorkerCommand::SetControlMode,
            _ => return Err(HandoffError),
        };
        validate_command(&fields, command)?;
        Ok(Self { command, fields })
    }

    /// The validated command; not a claim of capability authorization.
    pub fn command(&self) -> WorkerCommand {
        self.command
    }

    /// Read validated fields without exposing any mutable decoder state.
    pub fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }
}

fn validate_command(
    fields: &Map<String, Value>,
    command: WorkerCommand,
) -> Result<(), HandoffError> {
    let mut allowed = HEADER.to_vec();
    if command != WorkerCommand::Probe {
        allowed.push("worker_boot_id");
        uuid(fields, "worker_boot_id")?;
    }
    if matches!(
        command,
        WorkerCommand::Dispatch | WorkerCommand::Lookup | WorkerCommand::Acknowledge
    ) {
        allowed.extend_from_slice(TUPLE);
        tuple(fields)?;
    }
    let scope = match command {
        WorkerCommand::Probe => "probe",
        WorkerCommand::Dispatch => {
            allowed.extend_from_slice(&["mode_sequence", "operation", "parameters"]);
            positive(fields, "mode_sequence")?;
            constant(fields, "operation", "runtime_v3_episode")?;
            if !fields
                .get("parameters")
                .and_then(Value::as_object)
                .is_some_and(Map::is_empty)
            {
                return Err(HandoffError);
            }
            "dispatch"
        }
        WorkerCommand::Lookup => "lookup",
        WorkerCommand::Acknowledge => {
            allowed.push("terminal_digest");
            digest(fields, "terminal_digest")?;
            "acknowledge"
        }
        WorkerCommand::SetControlMode => {
            allowed.extend_from_slice(CONTROL);
            identity(fields, "deployment_id")?;
            identity(fields, "worker_owner_id")?;
            digest(fields, "worker_profile_digest")?;
            positive(fields, "mode_sequence")?;
            if !matches!(
                string(fields, "mode")?,
                "running" | "paused" | "draining" | "stopped"
            ) {
                return Err(HandoffError);
            }
            "control"
        }
    };
    constant(fields, "scope", scope)?;
    if fields.len() != allowed.len() || fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(HandoffError);
    }
    Ok(())
}

pub(super) fn tuple(fields: &Map<String, Value>) -> Result<(), HandoffError> {
    let ids = ["handoff_id", "run_id", "episode_id", "trajectory_id"];
    for (index, name) in ids.iter().enumerate() {
        uuid(fields, name)?;
        for prior in &ids[..index] {
            if fields[*name] == fields[*prior] {
                return Err(HandoffError);
            }
        }
    }
    identity(fields, "deployment_id")?;
    identity(fields, "worker_owner_id")?;
    for name in ["job_id", "attempt_id"] {
        let value = string(fields, name)?;
        if value.is_empty() || value.len() > 128 || value.bytes().any(|b| b < 32 || b == 127) {
            return Err(HandoffError);
        }
    }
    digest(fields, "worker_profile_digest")?;
    positive(fields, "attempt_number")?;
    constant(fields, "payload_digest", PAYLOAD_DIGEST)
}

pub(super) fn string<'a>(
    fields: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, HandoffError> {
    fields.get(name).and_then(Value::as_str).ok_or(HandoffError)
}

fn constant(fields: &Map<String, Value>, name: &str, expected: &str) -> Result<(), HandoffError> {
    if string(fields, name)? != expected {
        return Err(HandoffError);
    }
    Ok(())
}

fn positive(fields: &Map<String, Value>, name: &str) -> Result<(), HandoffError> {
    if fields
        .get(name)
        .and_then(Value::as_u64)
        .is_none_or(|value| value == 0)
    {
        return Err(HandoffError);
    }
    Ok(())
}

pub(super) fn uuid(fields: &Map<String, Value>, name: &str) -> Result<(), HandoffError> {
    let value = string(fields, name)?;
    let parsed = uuid::Uuid::parse_str(value).map_err(|_| HandoffError)?;
    if parsed.get_version_num() != 4
        || parsed.get_variant() != uuid::Variant::RFC4122
        || parsed.to_string() != value
    {
        return Err(HandoffError);
    }
    Ok(())
}

pub(super) fn digest(fields: &Map<String, Value>, name: &str) -> Result<(), HandoffError> {
    let value = string(fields, name)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(HandoffError);
    }
    Ok(())
}

pub(super) fn identity(fields: &Map<String, Value>, name: &str) -> Result<(), HandoffError> {
    let value = string(fields, name)?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(HandoffError);
    }
    Ok(())
}
