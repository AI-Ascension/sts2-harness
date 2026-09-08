// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::request::{CONTROL, HEADER, TUPLE, digest, identity, uuid};
use super::{HandoffError, MAX_FRAME_BYTES, TerminalRecord, WorkerCommand, WorkerRequest};

/// Probe metadata comes from the worker's pinned local configuration.
pub struct ProbeReply {
    pub deployment_id: String,
    pub worker_owner_id: String,
    pub worker_profile_digest: String,
    pub release_digest: String,
    pub config_digest: String,
    pub ready: bool,
}

/// Dispatch terminal statuses cannot omit their retained receipt.
pub enum DispatchReply {
    Accepted,
    Busy,
    Rejected,
    AlreadyCompleted(TerminalRecord),
    Terminal(TerminalRecord),
}

/// Unknown is not terminal and does not grant permission to redispatch.
pub enum LookupReply {
    Running,
    Unknown,
    Rejected,
    Terminal(TerminalRecord),
}

/// Acknowledgment reflects a durable store transition, not a socket write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcknowledgmentStatus {
    Acknowledged,
    AlreadyAcknowledged,
    Conflict,
    Rejected,
}

/// A response constructor cannot substitute a different command or tuple.
pub enum WorkerReply {
    Probe(ProbeReply),
    Dispatch(DispatchReply),
    Lookup(LookupReply),
    Acknowledge(AcknowledgmentStatus),
    Control { accepted: bool },
}

impl WorkerRequest {
    /// Encode a response echoing this request's exact correlation and tuple.
    /// The endpoint must authenticate the peer and persist transitions first.
    pub fn encode_response(
        &self,
        worker_boot_id: &str,
        reply: WorkerReply,
    ) -> Result<Vec<u8>, HandoffError> {
        let mut fields = Map::new();
        copy_fields(&mut fields, self, HEADER)?;
        fields.insert("direction".into(), Value::from("response"));
        fields.insert("worker_boot_id".into(), Value::from(worker_boot_id));
        uuid(&fields, "worker_boot_id")?;
        if self.command() != WorkerCommand::Probe
            && self.fields().get("worker_boot_id") != fields.get("worker_boot_id")
        {
            return Err(HandoffError);
        }
        match (self.command(), reply) {
            (WorkerCommand::Probe, WorkerReply::Probe(probe)) => install_probe(&mut fields, probe)?,
            (WorkerCommand::Dispatch, WorkerReply::Dispatch(reply)) => {
                copy_fields(&mut fields, self, TUPLE)?;
                let (status, terminal) = match reply {
                    DispatchReply::Accepted => ("accepted", None),
                    DispatchReply::Busy => ("busy", None),
                    DispatchReply::Rejected => ("rejected", None),
                    DispatchReply::AlreadyCompleted(terminal) => {
                        ("already_completed", Some(terminal))
                    }
                    DispatchReply::Terminal(terminal) => ("terminal", Some(terminal)),
                };
                install_terminal(&mut fields, self, status, terminal)?;
            }
            (WorkerCommand::Lookup, WorkerReply::Lookup(reply)) => {
                copy_fields(&mut fields, self, TUPLE)?;
                let (status, terminal) = match reply {
                    LookupReply::Running => ("running", None),
                    LookupReply::Unknown => ("unknown", None),
                    LookupReply::Rejected => ("rejected", None),
                    LookupReply::Terminal(terminal) => ("terminal", Some(terminal)),
                };
                install_terminal(&mut fields, self, status, terminal)?;
            }
            (WorkerCommand::Acknowledge, WorkerReply::Acknowledge(status)) => {
                copy_fields(&mut fields, self, TUPLE)?;
                fields.insert(
                    "status".into(),
                    Value::from(match status {
                        AcknowledgmentStatus::Acknowledged => "acknowledged",
                        AcknowledgmentStatus::AlreadyAcknowledged => "already_acknowledged",
                        AcknowledgmentStatus::Conflict => "conflict",
                        AcknowledgmentStatus::Rejected => "rejected",
                    }),
                );
            }
            (WorkerCommand::SetControlMode, WorkerReply::Control { accepted }) => {
                copy_fields(&mut fields, self, CONTROL)?;
                fields.insert(
                    "status".into(),
                    Value::from(if accepted { "accepted" } else { "rejected" }),
                );
            }
            _ => return Err(HandoffError),
        }
        let bytes = serde_json::to_vec(&fields).map_err(|_| HandoffError)?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(HandoffError);
        }
        Ok(bytes)
    }
}

fn copy_fields(
    fields: &mut Map<String, Value>,
    request: &WorkerRequest,
    names: &[&str],
) -> Result<(), HandoffError> {
    for name in names {
        fields.insert(
            (*name).to_owned(),
            request.fields().get(*name).ok_or(HandoffError)?.clone(),
        );
    }
    Ok(())
}

fn install_probe(fields: &mut Map<String, Value>, probe: ProbeReply) -> Result<(), HandoffError> {
    for (name, value) in [
        ("deployment_id", probe.deployment_id),
        ("worker_owner_id", probe.worker_owner_id),
        ("worker_profile_digest", probe.worker_profile_digest),
        ("release_digest", probe.release_digest),
        ("config_digest", probe.config_digest),
    ] {
        fields.insert(name.into(), Value::from(value));
    }
    identity(fields, "deployment_id")?;
    identity(fields, "worker_owner_id")?;
    for name in ["worker_profile_digest", "release_digest", "config_digest"] {
        digest(fields, name)?;
    }
    fields.insert("ready".into(), Value::from(probe.ready));
    fields.insert("admitting".into(), Value::Bool(false));
    Ok(())
}

fn install_terminal(
    fields: &mut Map<String, Value>,
    request: &WorkerRequest,
    status: &str,
    terminal: Option<TerminalRecord>,
) -> Result<(), HandoffError> {
    let terminal = match terminal {
        Some(terminal) => {
            if !terminal.matches(request) {
                return Err(HandoffError);
            }
            terminal.value()
        }
        None => Value::Null,
    };
    fields.insert("status".into(), Value::from(status));
    fields.insert("terminal".into(), terminal);
    Ok(())
}
