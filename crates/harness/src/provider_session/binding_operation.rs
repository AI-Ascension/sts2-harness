// SPDX-License-Identifier: MIT

use super::common::{
    MAX_DEPENDENCIES, SESSION_BINDING_SCHEMA, SESSION_OPERATION_SCHEMA, SessionError, digest,
    unique_ids, valid_digest, valid_id, valid_timestamp,
};
use super::scope_policy::SessionScope;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPurpose {
    Executable,
    Evaluation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingState {
    Candidate,
    Held,
    Active,
    Recovering,
    Quarantined,
    Retired,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCoverage {
    ApplicationManifestVerified,
    ReportedPartial,
    Unknown,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBinding {
    pub schema: String,
    pub binding_id: String,
    pub scope: SessionScope,
    pub branch_id: String,
    pub purpose: SessionPurpose,
    pub state: BindingState,
    pub native_thread_ref: String,
    pub credential_realm_ref: String,
    pub profile_sha256: String,
    pub owner_epoch: u64,
    pub session_epoch: u64,
    pub history_epoch: u64,
    pub compaction_epoch: u64,
    pub dependency_ids: Vec<String>,
    pub continuity_sha256: String,
    pub history_coverage: HistoryCoverage,
    pub expires_at: String,
    pub game_dispatch_capability: bool,
}

impl SessionBinding {
    pub fn candidate(
        binding_id: impl Into<String>,
        scope: SessionScope,
        branch_id: impl Into<String>,
        purpose: SessionPurpose,
        credential_realm_ref: impl Into<String>,
        profile_sha256: impl Into<String>,
        expires_at: impl Into<String>,
    ) -> Result<Self, SessionError> {
        let binding_id = binding_id.into();
        let branch_id = branch_id.into();
        let credential_realm_ref = credential_realm_ref.into();
        let profile_sha256 = profile_sha256.into();
        let value = Self {
            schema: SESSION_BINDING_SCHEMA.to_owned(),
            native_thread_ref: format!("pending-{binding_id}"),
            binding_id,
            scope,
            branch_id,
            purpose,
            state: BindingState::Candidate,
            credential_realm_ref,
            profile_sha256,
            owner_epoch: 1,
            session_epoch: 1,
            history_epoch: 0,
            compaction_epoch: 0,
            dependency_ids: Vec::new(),
            continuity_sha256: digest(b"empty-provider-history"),
            history_coverage: HistoryCoverage::Unknown,
            expires_at: expires_at.into(),
            game_dispatch_capability: false,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), SessionError> {
        if self.schema != SESSION_BINDING_SCHEMA
            || !valid_id(&self.binding_id)
            || !self.scope.valid()
            || !valid_id(&self.branch_id)
            || !valid_id(&self.native_thread_ref)
            || !valid_id(&self.credential_realm_ref)
            || !valid_digest(&self.profile_sha256)
            || !valid_digest(&self.continuity_sha256)
            || self.owner_epoch == 0
            || self.session_epoch == 0
            || self.dependency_ids.len() > MAX_DEPENDENCIES
            || !unique_ids(&self.dependency_ids)
            || !valid_timestamp(&self.expires_at)
            || (self.game_dispatch_capability && self.purpose != SessionPurpose::Executable)
            || (self.game_dispatch_capability && self.state != BindingState::Active)
            || (matches!(self.state, BindingState::Retired | BindingState::Closed)
                && self.game_dispatch_capability)
        {
            return Err(SessionError::InvalidBinding);
        }
        Ok(())
    }

    #[must_use]
    pub fn executable(&self) -> bool {
        matches!(self.purpose, SessionPurpose::Executable)
            && matches!(self.state, BindingState::Active)
            && self.game_dispatch_capability
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeOperationKind {
    CreateCandidate,
    Reconnect,
    Refresh,
    Fork,
    Compact,
    Retire,
    Cleanup,
    Turn,
    Interrupt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeOperationState {
    Planned,
    IntentPersisted,
    Sent,
    Acknowledged,
    Completed,
    Rejected,
    Unknown,
    Cancelled,
    Quarantined,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeOperation {
    pub schema: String,
    pub operation_id: String,
    pub scope: SessionScope,
    pub binding_id: String,
    pub kind: NativeOperationKind,
    pub idempotency_key: String,
    pub request_sha256: String,
    pub state: NativeOperationState,
    pub owner_epoch: u64,
    pub session_epoch: u64,
    pub generation_permission: bool,
    pub generation_class: bool,
    pub automatic_retry: bool,
    pub auto_resume: bool,
    pub game_effects: u64,
    pub terminal_evidence_ref: Option<String>,
}

impl NativeOperation {
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.schema != SESSION_OPERATION_SCHEMA
            || !valid_id(&self.operation_id)
            || !self.scope.valid()
            || !valid_id(&self.binding_id)
            || !valid_id(&self.idempotency_key)
            || !valid_digest(&self.request_sha256)
            || self.owner_epoch == 0
            || self.session_epoch == 0
            || self.automatic_retry
            || self.auto_resume
            || self.game_effects != 0
            || (self.generation_class && !self.generation_permission)
            || self
                .terminal_evidence_ref
                .as_ref()
                .is_some_and(|v| !valid_id(v))
        {
            return Err(SessionError::InvalidOperation);
        }
        Ok(())
    }
}
