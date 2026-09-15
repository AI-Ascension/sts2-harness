// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::{ContextOwnerBinding, StoreError};

pub(crate) const MAX_BINDING_HISTORY_BYTES: usize = 16_384;
pub(crate) const MAX_BINDING_HISTORY_PER_RUN: usize = 256;
pub(crate) const CONTEXT_BINDING_HISTORY_SCHEMA: &str =
    "ascension.harness.context-binding-history.v1";

/// Historical owner response committed with one management command result.
///
/// All binding grants, epochs and continuity flags describe the original
/// invocation only. They confer no current permission, attachment, freshness,
/// recovery or execution authority. This library record is not itself an HTTP contract; the
/// separately versioned [`RecordedContextBindingView`] is the read-only HTTP projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedContextBinding {
    pub(crate) schema_version: String,
    pub binding: ContextOwnerBinding,
    pub command_id: String,
    pub run_revision: u64,
    pub(crate) subject: String,
}

/// HTTP-visible schema for the bounded recorded-binding projection.
pub const RECORDED_CONTEXT_BINDING_VIEW_SCHEMA: &str =
    "ascension.harness.recorded-context-binding-view.v1";

/// Bounded, versioned HTTP projection of one recorded context-owner binding.
///
/// Observation only. The projected grants, epochs and continuity flags describe
/// the original invocation and confer no current permission, attachment,
/// freshness, recovery or execution authority. The originating subject is
/// deliberately not projected, and no content bytes are included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RecordedContextBindingView {
    pub schema_version: String,
    pub command_id: String,
    pub run_revision: u64,
    pub binding: ContextOwnerBinding,
}

impl From<&RecordedContextBinding> for RecordedContextBindingView {
    fn from(record: &RecordedContextBinding) -> Self {
        Self {
            schema_version: RECORDED_CONTEXT_BINDING_VIEW_SCHEMA.to_owned(),
            command_id: record.command_id.clone(),
            run_revision: record.run_revision,
            binding: record.binding.clone(),
        }
    }
}

impl RecordedContextBinding {
    pub(crate) fn validate(&self) -> Result<(), StoreError> {
        self.binding.validate(None).map_err(|_| invalid())?;
        super::validate_identifier("command_id", &self.command_id).map_err(|_| invalid())?;
        super::validate_identifier("subject", &self.subject).map_err(|_| invalid())?;
        if self.schema_version != CONTEXT_BINDING_HISTORY_SCHEMA
            || self.run_revision == 0
            || !matches!(self.binding.state, super::ContextBindingState::Available)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(crate) fn invalid() -> StoreError {
    StoreError::new(
        "context_history_invalid",
        "historical context binding is invalid or inconsistent",
    )
}
