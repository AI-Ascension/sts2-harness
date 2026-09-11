// SPDX-License-Identifier: MIT

use super::binding_operation::{HistoryCoverage, SessionBinding};
use super::common::{
    MAX_DEPENDENCIES, MAX_OUTPUT_SCHEMA_BYTES, MAX_SUFFIX_BYTES, SESSION_PREPARED_SCHEMA,
    SessionError, digest, unique_ids, valid_digest, valid_id, valid_timestamp,
};
use super::scope_policy::{ContinuityMode, SessionScope};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryItem {
    pub item_ref: String,
    pub turn_ref: String,
    pub sequence: u64,
    pub kind: HistoryItemKind,
    pub content_ref: Option<String>,
    #[serde(default)]
    pub redacted: bool,
}

impl HistoryItem {
    pub fn validate(&self) -> Result<(), SessionError> {
        if !valid_id(&self.item_ref)
            || !valid_id(&self.turn_ref)
            || self
                .content_ref
                .as_ref()
                .is_some_and(|content_ref| !valid_id(content_ref))
        {
            return Err(SessionError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryItemKind {
    UserInput,
    ValidatedDecision,
    CompactionMetadata,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryView {
    pub schema: String,
    pub view_id: String,
    pub binding_id: String,
    pub scope: SessionScope,
    pub history_epoch: u64,
    pub watermark: u64,
    pub coverage: HistoryCoverageView,
    pub effective_context_coverage: &'static str,
    pub items: Vec<HistoryItem>,
    pub known_total_items: Option<usize>,
    pub next_cursor: Option<String>,
    pub read_started_turn: bool,
    pub expires_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCoverageView {
    CompleteAtWatermark,
    Partial,
    Redacted,
    Unavailable,
    Unknown,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedSessionTurn {
    pub schema: String,
    pub prepared_id: String,
    pub scope: SessionScope,
    pub binding_id: String,
    pub phase2_preview_id: String,
    pub phase2_revision_id: String,
    pub phase3_selection_id: String,
    pub held_boundary_ref: String,
    pub owner_epoch: u64,
    pub control_generation: u64,
    pub session_epoch: u64,
    pub history_epoch: u64,
    pub compaction_epoch: u64,
    pub auth_epoch: u64,
    pub revocation_epoch: u64,
    pub profile_sha256: String,
    pub continuity_sha256: String,
    pub suffix_ref: String,
    pub suffix_sha256: String,
    pub output_schema_ref: String,
    pub output_schema_sha256: String,
    pub protected_ref: String,
    pub protected_sha256: String,
    pub dependency_ids: Vec<String>,
    pub continuity_mode: ContinuityMode,
    pub provider_internal_context: &'static str,
    pub history_coverage: HistoryCoverage,
    pub effect_class: &'static str,
    pub expires_at: String,
    #[serde(skip)]
    pub suffix: Vec<u8>,
    #[serde(skip)]
    pub output_schema: Vec<u8>,
    #[serde(skip)]
    pub protected: Vec<u8>,
}

impl PreparedSessionTurn {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prepared_id: impl Into<String>,
        scope: SessionScope,
        binding: &SessionBinding,
        phase2_preview_id: impl Into<String>,
        phase2_revision_id: impl Into<String>,
        phase3_selection_id: impl Into<String>,
        held_boundary_ref: impl Into<String>,
        suffix: Vec<u8>,
        output_schema: Vec<u8>,
        protected: Vec<u8>,
        dependencies: Vec<String>,
        continuity_mode: ContinuityMode,
        expires_at: impl Into<String>,
    ) -> Result<Self, SessionError> {
        if scope != binding.scope {
            return Err(SessionError::InvalidScope);
        }
        if suffix.is_empty()
            || suffix.len() > MAX_SUFFIX_BYTES
            || output_schema.is_empty()
            || output_schema.len() > MAX_OUTPUT_SCHEMA_BYTES
            || protected.len() > MAX_SUFFIX_BYTES
            || dependencies.len() > MAX_DEPENDENCIES
            || !unique_ids(&dependencies)
        {
            return Err(SessionError::Capacity);
        }
        let suffix_sha256 = digest(&suffix);
        let output_schema_sha256 = digest(&output_schema);
        let protected_sha256 = digest(&protected);
        let suffix_ref = format!("suffix-{}", &suffix_sha256[..16]);
        let output_schema_ref = format!("schema-{}", &output_schema_sha256[..16]);
        let protected_ref = format!("protected-{}", &protected_sha256[..16]);
        let value = Self {
            schema: SESSION_PREPARED_SCHEMA.to_owned(),
            prepared_id: prepared_id.into(),
            scope,
            binding_id: binding.binding_id.clone(),
            phase2_preview_id: phase2_preview_id.into(),
            phase2_revision_id: phase2_revision_id.into(),
            phase3_selection_id: phase3_selection_id.into(),
            held_boundary_ref: held_boundary_ref.into(),
            owner_epoch: binding.owner_epoch,
            control_generation: 1,
            session_epoch: binding.session_epoch,
            history_epoch: binding.history_epoch,
            compaction_epoch: binding.compaction_epoch,
            auth_epoch: 1,
            revocation_epoch: 0,
            profile_sha256: binding.profile_sha256.clone(),
            continuity_sha256: binding.continuity_sha256.clone(),
            suffix_ref,
            suffix_sha256,
            output_schema_ref,
            output_schema_sha256,
            protected_ref,
            protected_sha256,
            dependency_ids: dependencies,
            continuity_mode,
            provider_internal_context: "unexposed",
            history_coverage: binding.history_coverage,
            effect_class: "local_preparation_only",
            expires_at: expires_at.into(),
            suffix,
            output_schema,
            protected,
        };
        value.validate().map(|()| value)
    }

    pub fn validate(&self) -> Result<(), SessionError> {
        if self.schema != SESSION_PREPARED_SCHEMA
            || !valid_id(&self.prepared_id)
            || !self.scope.valid()
            || !valid_id(&self.binding_id)
            || !valid_id(&self.phase2_preview_id)
            || !valid_id(&self.phase2_revision_id)
            || !valid_id(&self.phase3_selection_id)
            || !valid_id(&self.held_boundary_ref)
            || self.owner_epoch == 0
            || self.session_epoch == 0
            || !valid_digest(&self.profile_sha256)
            || !valid_digest(&self.continuity_sha256)
            || !valid_digest(&self.suffix_sha256)
            || !valid_digest(&self.output_schema_sha256)
            || !valid_digest(&self.protected_sha256)
            || !valid_id(&self.suffix_ref)
            || !valid_id(&self.output_schema_ref)
            || !valid_id(&self.protected_ref)
            || self.dependency_ids.len() > MAX_DEPENDENCIES
            || !unique_ids(&self.dependency_ids)
            || self.provider_internal_context != "unexposed"
            || self.effect_class != "local_preparation_only"
            || !valid_timestamp(&self.expires_at)
            || self.suffix.is_empty()
            || self.output_schema.is_empty()
            || self.suffix.len() > MAX_SUFFIX_BYTES
            || serde_json::from_slice::<serde_json::Value>(&self.output_schema)
                .ok()
                .and_then(|value| value.as_object().map(|_| ()))
                .is_none()
            || digest(&self.suffix) != self.suffix_sha256
            || digest(&self.output_schema) != self.output_schema_sha256
            || digest(&self.protected) != self.protected_sha256
        {
            return Err(SessionError::InvalidPrepared);
        }
        Ok(())
    }
}
