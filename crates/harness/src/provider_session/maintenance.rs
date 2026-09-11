// SPDX-License-Identifier: MIT

use super::common::{
    MAX_DEPENDENCIES, SESSION_COMPACTION_SCHEMA, SESSION_FORK_SCHEMA, SESSION_RETIREMENT_SCHEMA,
    unique_ids, valid_digest, valid_id,
};
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
    pub scheduler_after: String,
}

impl CompactionJob {
    pub fn validate(&self) -> Result<(), super::common::SessionError> {
        if self.schema != SESSION_COMPACTION_SCHEMA
            || !valid_id(&self.job_id)
            || !self.scope.valid()
            || !valid_id(&self.binding_id)
            || !valid_digest(&self.source_continuity_sha256)
            || self.dependency_ids.len() > MAX_DEPENDENCIES
            || !unique_ids(&self.dependency_ids)
            || self
                .budget_reservation_ref
                .as_ref()
                .is_some_and(|value| !valid_id(value))
            || self
                .terminal_evidence_ref
                .as_ref()
                .is_some_and(|value| !valid_id(value))
            || self.history_epoch_after.is_some_and(|epoch| epoch == 0)
            || self.automatic_adoption
            || self.game_effects != 0
            || self.scheduler_after != "held"
        {
            return Err(super::common::SessionError::InvalidRequest);
        }
        Ok(())
    }
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
    pub purpose: String,
    pub operation: ForkOperation,
    pub copies_native_history: bool,
    pub dependency_ids: Vec<String>,
    pub automatic_inference: bool,
    pub game_dispatch_capability: bool,
}

impl ForkPlan {
    pub fn validate(&self) -> Result<(), super::common::SessionError> {
        if self.schema != SESSION_FORK_SCHEMA
            || !valid_id(&self.fork_plan_id)
            || !self.scope.valid()
            || !valid_id(&self.source_binding_id)
            || !valid_digest(&self.source_continuity_sha256)
            || !valid_id(&self.cutoff_turn_ref)
            || !valid_id(&self.target_binding_id)
            || self.purpose != "evaluation"
            || self.dependency_ids.len() > MAX_DEPENDENCIES
            || !unique_ids(&self.dependency_ids)
            || self.automatic_inference
            || self.game_dispatch_capability
            || (matches!(self.operation, ForkOperation::CleanRehydration)
                && (self.native_cutoff_verified || self.copies_native_history))
            || (matches!(self.operation, ForkOperation::NativeFork)
                && (!self.native_cutoff_verified || !self.copies_native_history))
        {
            return Err(super::common::SessionError::InvalidRequest);
        }
        Ok(())
    }
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

impl Retirement {
    pub fn validate(&self) -> Result<(), super::common::SessionError> {
        if self.schema != SESSION_RETIREMENT_SCHEMA
            || !valid_id(&self.retirement_id)
            || !self.scope.valid()
            || self.binding_ids.len() > MAX_DEPENDENCIES
            || !unique_ids(&self.binding_ids)
            || self.revoked_source_ids.len() > MAX_DEPENDENCIES
            || !unique_ids(&self.revoked_source_ids)
            || self.affected_native_binding_ids.len() > MAX_DEPENDENCIES
            || !unique_ids(&self.affected_native_binding_ids)
            || !self.admission_denied
            || self.auto_resume
        {
            return Err(super::common::SessionError::InvalidRequest);
        }
        Ok(())
    }
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
