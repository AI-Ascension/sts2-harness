// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

pub const CONTEXT_DRAFT_SCHEMA: &str = "ascension.context-control.draft.v1";
pub const CONTROL_JOURNAL_SCHEMA: &str = "ascension.context-control.journal.v1";
pub const MAX_CONTEXT_ITEMS: usize = 64;
pub const MAX_CONTEXT_NOTES: usize = 16;
pub const MAX_NOTE_BYTES: usize = 4 * 1024;
pub const MAX_OBJECTIVE_BYTES: usize = 512;
pub const MAX_CONTEXT_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ManagementProfile {
    Legacy,
    Enabled,
}

impl ManagementProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Enabled => "management-enabled",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextItemRef {
    pub item_id: String,
    pub version: u64,
    pub sha256: String,
}

impl ContextItemRef {
    pub fn valid(&self) -> bool {
        valid_id(&self.item_id)
            && self.version > 0
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            && self.sha256.bytes().all(|byte| !byte.is_ascii_uppercase())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub reference: ContextItemRef,
    pub kind: String,
    pub bytes: Vec<u8>,
    pub protected: bool,
    pub expires_at: u64,
}

impl ContextItem {
    pub fn editable(&self, now: u64) -> bool {
        !self.protected && !self.bytes.is_empty() && self.expires_at > now
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextNote {
    pub reference: ContextItemRef,
    pub attributed_to: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextDraft {
    pub schema: String,
    pub draft_id: String,
    pub version: u64,
    pub base_revision_id: String,
    pub selected_items: Vec<ContextItemRef>,
    pub pinned_item_ids: Vec<String>,
    pub notes: Vec<ContextNote>,
    pub objective: Option<ContextItemRef>,
    pub author_ref: String,
}

impl ContextDraft {
    pub fn new(draft_id: impl Into<String>, base_revision_id: impl Into<String>) -> Self {
        Self {
            schema: CONTEXT_DRAFT_SCHEMA.to_owned(),
            draft_id: draft_id.into(),
            version: 1,
            base_revision_id: base_revision_id.into(),
            selected_items: Vec::new(),
            pinned_item_ids: Vec::new(),
            notes: Vec::new(),
            objective: None,
            author_ref: "operator".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextBoundary {
    pub run_id: String,
    pub episode_id: String,
    pub agent_id: String,
    pub state_id: String,
    pub generation: u64,
    pub observation_sha256: String,
    pub catalog_sha256: String,
    pub adapter_revision: String,
    pub model_revision: String,
    pub configuration_sha256: String,
    pub output_schema_sha256: String,
    pub controller_epoch: u64,
    pub gate_epoch: u64,
    pub control_version: u64,
}

impl ContextBoundary {
    pub fn external_eq(&self, other: &Self) -> bool {
        self.run_id == other.run_id
            && self.episode_id == other.episode_id
            && self.agent_id == other.agent_id
            && self.state_id == other.state_id
            && self.generation == other.generation
            && self.observation_sha256 == other.observation_sha256
            && self.catalog_sha256 == other.catalog_sha256
            && self.adapter_revision == other.adapter_revision
            && self.model_revision == other.model_revision
            && self.configuration_sha256 == other.configuration_sha256
            && self.output_schema_sha256 == other.output_schema_sha256
            && self.controller_epoch == other.controller_epoch
            && self.gate_epoch == other.gate_epoch
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}
