// SPDX-License-Identifier: MIT

use super::scope_policy::SessionScope;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionJob {
    pub schema: String,
    pub job_id: String,
    pub scope: SessionScope,
    pub binding_id: String,
    pub source_history_epoch: u64,
    pub source_continuity_sha256: String,
    pub dependency_ids: Vec<String>,
    pub state: CompactionState,
    pub generation_permission: bool,
    pub budget_reservation_ref: Option<String>,
    pub ack_received: bool,
    pub terminal_evidence_ref: Option<String>,
    pub history_epoch_after: Option<u64>,
    pub representation: CompactionRepresentation,
    pub automatic_adoption: bool,
    pub game_effects: u64,
    pub scheduler_after: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionState {
    Planned,
    Sent,
    Acknowledged,
    Transforming,
    Completed,
    Failed,
    Unknown,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionRepresentation {
    Pending,
    OpaqueNative,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkPlan {
    pub schema: String,
    pub fork_plan_id: String,
    pub scope: SessionScope,
    pub source_binding_id: String,
    pub source_history_epoch: u64,
    pub source_continuity_sha256: String,
    pub cutoff_turn_ref: String,
    pub cutoff_sequence: u64,
    pub native_cutoff_verified: bool,
    pub target_binding_id: String,
    pub purpose: &'static str,
    pub operation: ForkOperation,
    pub copies_native_history: bool,
    pub dependency_ids: Vec<String>,
    pub automatic_inference: bool,
    pub game_dispatch_capability: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkOperation {
    NativeFork,
    CleanRehydration,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retirement {
    pub schema: String,
    pub retirement_id: String,
    pub scope: SessionScope,
    pub binding_ids: Vec<String>,
    pub revoked_source_ids: Vec<String>,
    pub admission_denied: bool,
    pub local_status: RetirementLocalStatus,
    pub native_cleanup_status: NativeCleanupStatus,
    pub remote_erasure_status: RemoteErasureStatus,
    pub cascade_verified: bool,
    pub affected_native_binding_ids: Vec<String>,
    pub auto_resume: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetirementLocalStatus {
    Denied,
    CleanupPending,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeCleanupStatus {
    NotRequested,
    Pending,
    Completed,
    Unsupported,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteErasureStatus {
    NotRequested,
    Unverified,
    ProviderReported,
}
