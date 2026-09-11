// SPDX-License-Identifier: MIT

use super::scope_policy::SessionScope;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEvent {
    pub schema: String,
    pub event_id: String,
    pub scope: SessionScope,
    pub binding_id: String,
    pub operation_id: Option<String>,
    pub local_ingest_sequence: u64,
    pub sequence_origin: String,
    pub owner_epoch: u64,
    pub session_epoch: u64,
    pub kind: SessionEventKind,
    pub metadata: EventMetadata,
    pub starts_inference: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventMetadata {
    pub status: SessionEventStatus,
    pub count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    CandidateCreated,
    ReconnectedHeld,
    TurnObserved,
    TransformObserved,
    Retired,
    HistoryGap,
    OperationUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventStatus {
    Observed,
    Partial,
    Unknown,
    Denied,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageMeasurement {
    pub schema: String,
    pub measurement_id: String,
    pub binding_id: String,
    pub scope: SessionScope,
    pub native_turn_ref: Option<String>,
    pub measurement_scope: UsageScope,
    pub source: UsageSource,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub effective_context_tokens: Option<u64>,
    pub effective_context_quality: UsageQuality,
    pub remaining_context_tokens: Option<u64>,
    pub counts_reasoning_within_output: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageScope {
    LastTurn,
    CumulativeSession,
    CompactionJob,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    SyntheticPeer,
    NativeReported,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageQuality {
    Reported,
    Estimate,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reconciliation {
    pub schema: String,
    pub reconciliation_id: String,
    pub operation_id: String,
    pub binding_id: String,
    pub scope: SessionScope,
    pub outcome: ReconciliationOutcome,
    pub evidence_refs: Vec<String>,
    pub native_turn_ref: Option<String>,
    pub auto_retry_generation: bool,
    pub scheduler_after: &'static str,
    pub new_generation_calls: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationOutcome {
    ProvenNotSent,
    ObservedCompletion,
    Ambiguous,
    Quarantined,
}
