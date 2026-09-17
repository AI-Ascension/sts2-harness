// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::target_admission::{ExecutionMode, TargetAdmissionBinding};

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

/// Typed recovery admission exposed to supervision consumers.
///
/// This is deliberately a control-plane result. It contains no workflow
/// cursor, action identity, provider output, or command authority.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecoveryAdmission {
    NoPendingEffects,
    Reconciling,
    SafelyResumable { capability: String },
    NeedsOperator,
    TerminalVerifiedCleanup,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission: Option<TargetAdmissionBinding>,
    /// The execution mode the serving composition declared for this run.
    ///
    /// Internal only: this key is deliberately kept off the serialized
    /// run-snapshot contract. The pinned Studio consumer parses that object with
    /// a closed (`strict`) schema that does not list `execution_mode`, so a new
    /// key would break every run read. Extending a closed public schema requires
    /// explicit version/capability negotiation plus coordinated
    /// producer/consumer pin updates (issue #94). Until that negotiation lands,
    /// the harness keeps the in-process report solely to refuse a port whose
    /// declared mode contradicts the admitted target, and never emits it.
    ///
    /// `default` keeps records written before mode reporting deserializable.
    ///
    /// Per #247 AC1: `skip_serializing` here is a deliberate wire-contract omission of a coarse
    /// `Synthetic`/`Live` classification, not a secrecy boundary over private bytes, so the derived
    /// `Debug` needs no allowlist. The value is already derivable from the admitted target, and the
    /// dense rationale above records why the key is withheld from the pinned consumer's closed
    /// schema.
    #[serde(default, skip_serializing)]
    pub execution_mode: Option<ExecutionMode>,
}

/// A scoped, redacted association between the current workflow cursor and its
/// context evidence. Content bytes, provider credentials, and raw provider
/// payloads are intentionally absent from this management contract.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextAssociation {
    pub schema_version: String,
    pub workflow: ContextWorkflowIdentity,
    pub context: ContextAssociationContext,
    pub capture: ContextCaptureEvidence,
    pub capabilities: ContextInspectionCapabilities,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextWorkflowIdentity {
    pub workflow_run_id: String,
    pub definition_digest: String,
    pub graph_id: String,
    pub node_id: String,
    pub node_execution_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextAvailability {
    Available,
    Unavailable,
    NotApplicable,
}

/// Per #247 AC1: `reason_code` is `skip_serializing_if`-gated, not `#[serde(skip)]`, so this type
/// is outside the AC1 sweep and needs no `Debug` allowlist. It is omitted only while `None` and
/// carries a bounded explanation code with no private payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextAssociationContext {
    pub availability: ContextAvailability,
    pub context_ref: Option<String>,
    pub run_id: Option<String>,
    pub episode_id: Option<String>,
    pub agent_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub approved_revision_id: Option<String>,
    pub plan_epoch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCaptureMode {
    Off,
    Metadata,
    Memory,
    Private,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCaptureState {
    NotCaptured,
    Prepared,
    InputWriteCompleted,
    ProviderReceiptReported,
    Unknown,
    Unavailable,
}

/// Per #247 AC1: `reason_code` is `skip_serializing_if`-gated, not `#[serde(skip)]`, so this type
/// is outside the AC1 sweep and needs no `Debug` allowlist. It is omitted only while `None` and
/// carries a bounded explanation code with no private payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextCaptureEvidence {
    pub mode: ContextCaptureMode,
    pub state: ContextCaptureState,
    pub attempt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextInspectionCapabilities {
    pub inspect_metadata: bool,
    pub read_retained_content: bool,
    pub edit_context: bool,
    pub control_context: bool,
    pub memory_search: bool,
    pub provider_session_inspect: bool,
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
