// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    Created,
    Validated,
    Running,
    WaitingForProvider,
    WaitingForGame,
    Pausing,
    Paused,
    Cancelling,
    Reconciling,
    Completed,
    Failed,
    Cancelled,
    NeedsOperator,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameOutcome {
    NotTerminal,
    Victory,
    Defeat,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupState {
    NotStarted,
    Pending,
    Complete,
    Failed,
    NeedsOperator,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub graph_id: String,
    pub node_id: String,
    pub node_execution_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PendingOperation {
    pub operation_id: String,
    #[serde(rename = "state")]
    pub classification: PendingOperationState,
    pub instance_id: String,
    pub original_generation: u64,
    pub payload_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingOperationState {
    Intent,
    Accepted,
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub provider_calls_consumed: u64,
    pub provider_calls_reserved: u64,
    pub node_steps_consumed: u64,
    pub replans_consumed: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunSnapshot {
    pub schema_version: String,
    pub workflow_run_id: String,
    pub definition_digest: String,
    pub run_revision: u64,
    pub status: WorkflowRunStatus,
    pub game_outcome: GameOutcome,
    pub cursor: Cursor,
    pub pending_operation: Option<PendingOperation>,
    pub budget: Budget,
    pub cleanup: CleanupState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    RunStarted,
    NodeEntered,
    NodeCompleted,
    PlanAccepted,
    PlanRejected,
    OperationIntent,
    OperationUnknown,
    OperationSettled,
    CommandRequested,
    CommandApplied,
    RunTerminal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventPayload {
    pub operation_id: Option<String>,
    pub classification: Option<EventClassification>,
    pub reason_code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventClassification {
    Accepted,
    Unknown,
    Settled,
    Rejected,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunEvent {
    pub schema_version: String,
    pub workflow_run_id: String,
    pub sequence: u64,
    pub run_revision: u64,
    pub event_type: EventType,
    pub definition_digest: String,
    pub node_execution_id: String,
    pub payload: EventPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity_digest: Option<String>,
}

impl RunEvent {
    pub(crate) fn seal_integrity(mut self) -> Result<Self, String> {
        self.integrity_digest = None;
        let bytes = serde_json::to_vec(&self).map_err(|error| error.to_string())?;
        self.integrity_digest = Some(hex_digest(&bytes));
        Ok(self)
    }

    #[must_use]
    pub(crate) fn integrity_valid(&self) -> bool {
        let Some(digest) = self.integrity_digest.as_deref() else {
            return false;
        };
        let mut unsigned = self.clone();
        unsigned.integrity_digest = None;
        serde_json::to_vec(&unsigned)
            .ok()
            .is_some_and(|bytes| digest == hex_digest(&bytes))
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventGap {
    pub requested_after_sequence: u64,
    pub oldest_sequence: u64,
    pub newest_sequence: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventPage {
    pub schema_version: String,
    pub workflow_run_id: String,
    pub after_sequence: u64,
    pub oldest_sequence: Option<u64>,
    pub newest_sequence: Option<u64>,
    pub next_after_sequence: u64,
    pub gap: Option<EventGap>,
    pub events: Vec<RunEvent>,
}

#[path = "contract_commands.rs"]
mod commands;

pub use commands::*;
