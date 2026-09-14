// SPDX-License-Identifier: MIT

//! Exact command and receipt records for delegated context control.

use super::*;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextControlCommandKind {
    Pause,
    Commit,
    Resume,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextControlCommand {
    Pause {
        idempotency_key: String,
        expected_control_version: u64,
    },
    Commit {
        idempotency_key: String,
        expected_control_version: u64,
        expected_revision_id: String,
        expected_boundary: ContextBoundary,
        preview_manifest_digest: String,
        approved_manifest_digest: String,
    },
    Resume {
        idempotency_key: String,
        expected_control_version: u64,
        expected_boundary: ContextBoundary,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextControlReceipt {
    pub schema_version: String,
    pub owner_id: String,
    pub invocation_id: String,
    pub binding_id: String,
    pub binding_digest: String,
    /// The command variant that produced this receipt. An effect string alone
    /// cannot prevent a receipt for one command from being replayed as another.
    pub command: ContextControlCommandKind,
    pub command_id: String,
    pub idempotency_key: String,
    pub effect: String,
    pub control_version: u64,
    pub plan_epoch: u64,
    pub controller_epoch: u64,
    pub gate_epoch: u64,
    /// Epochs alone do not identify an episode or agent, so retain the full
    /// boundary identity with every delegated effect.
    pub boundary: ContextBoundary,
    /// Commit-only identity. Pause and resume receipts must leave these fields
    /// absent.
    pub revision_id: Option<String>,
    pub preview_manifest_digest: Option<String>,
    pub approved_manifest_digest: Option<String>,
}

impl ContextControlReceipt {
    pub fn validate_for(
        &self,
        binding: &ContextOwnerBinding,
        command: &ContextControlCommand,
    ) -> Result<(), ManagementError> {
        binding.validate(None)?;
        if self.schema_version != CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION
            || self.owner_id != binding.owner_id
            || self.invocation_id != binding.invocation_id
            || self.binding_id != binding.binding_id
            || self.binding_digest != binding.binding_digest
            || self.control_version == 0
            || self.plan_epoch == 0
            || self.controller_epoch == 0
            || self.gate_epoch == 0
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_mismatch",
                "context control receipt is not bound to the requested owner invocation",
            ));
        }
        validate_boundary(&self.boundary)?;
        if self.boundary.controller_epoch != binding.boundary.controller_epoch
            || self.boundary.run_id != binding.boundary.run_id
            || self.boundary.episode_id != binding.boundary.episode_id
            || self.boundary.agent_id != binding.boundary.agent_id
            || self.boundary.state_id != binding.boundary.state_id
            || self.boundary.generation != binding.boundary.generation
            || self.boundary.observation_sha256 != binding.boundary.observation_sha256
            || self.boundary.catalog_sha256 != binding.boundary.catalog_sha256
            || self.boundary.adapter_revision != binding.boundary.adapter_revision
            || self.boundary.model_revision != binding.boundary.model_revision
            || self.boundary.configuration_sha256 != binding.boundary.configuration_sha256
            || self.boundary.output_schema_sha256 != binding.boundary.output_schema_sha256
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_boundary_identity",
                "context control receipt belongs to a foreign context boundary",
            ));
        }
        if self.controller_epoch != self.boundary.controller_epoch
            || self.gate_epoch != self.boundary.gate_epoch
            || self.control_version != self.boundary.control_version
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_boundary_epochs",
                "context control receipt epoch fields do not match its boundary",
            ));
        }
        validate_identifier("context_control_command_id", &self.command_id)?;
        validate_identifier("context_control_idempotency_key", &self.idempotency_key)?;
        validate_identifier("context_control_effect", &self.effect)?;
        validate_digest("context_binding_digest", &self.binding_digest)?;
        let (kind, idempotency_key, expected_control_version, boundary, revision, manifests) =
            match command {
                ContextControlCommand::Pause {
                    idempotency_key,
                    expected_control_version,
                } => (
                    ContextControlCommandKind::Pause,
                    idempotency_key,
                    *expected_control_version,
                    None,
                    None,
                    None,
                ),
                ContextControlCommand::Commit {
                    idempotency_key,
                    expected_control_version,
                    expected_revision_id,
                    expected_boundary,
                    preview_manifest_digest,
                    approved_manifest_digest,
                } => (
                    ContextControlCommandKind::Commit,
                    idempotency_key,
                    *expected_control_version,
                    Some(expected_boundary),
                    Some(expected_revision_id),
                    Some((preview_manifest_digest, approved_manifest_digest)),
                ),
                ContextControlCommand::Resume {
                    idempotency_key,
                    expected_control_version,
                    expected_boundary,
                } => (
                    ContextControlCommandKind::Resume,
                    idempotency_key,
                    *expected_control_version,
                    Some(expected_boundary),
                    None,
                    None,
                ),
            };
        if self.command != kind {
            return Err(ManagementError::conflict(
                "context_control_receipt_command",
                "context control receipt command variant does not match the request",
            ));
        }
        let expected_effect = match kind {
            ContextControlCommandKind::Pause => "pause_requested",
            ContextControlCommandKind::Commit => "revision_committed",
            ContextControlCommandKind::Resume => "resume_accepted",
        };
        if self.effect != expected_effect {
            return Err(ManagementError::conflict(
                "context_control_receipt_effect",
                "context control receipt effect does not match the requested command",
            ));
        }
        validate_identifier("context_control_idempotency_key", idempotency_key)?;
        if expected_control_version == 0
            || self.idempotency_key != *idempotency_key
            || self.control_version != expected_control_version
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_fence",
                "context control receipt does not satisfy the requested version fence",
            ));
        }
        if self.plan_epoch != binding.plan_epoch {
            return Err(ManagementError::conflict(
                "context_control_receipt_plan_epoch",
                "context control receipt belongs to a different context plan",
            ));
        }
        match (revision, manifests) {
            (Some(expected_revision_id), Some((preview, approved))) => {
                validate_identifier("context_control_revision_id", expected_revision_id)?;
                validate_digest("context_control_preview_manifest", preview)?;
                validate_digest("context_control_approved_manifest", approved)?;
                if self.revision_id.as_deref() != Some(expected_revision_id)
                    || self.preview_manifest_digest.as_deref() != Some(preview)
                    || self.approved_manifest_digest.as_deref() != Some(approved)
                {
                    return Err(ManagementError::conflict(
                        "context_control_receipt_commit_identity",
                        "context control commit receipt does not match its revision or manifests",
                    ));
                }
            }
            (None, None)
                if self.revision_id.is_none()
                    && self.preview_manifest_digest.is_none()
                    && self.approved_manifest_digest.is_none() => {}
            _ => {
                return Err(ManagementError::conflict(
                    "context_control_receipt_commit_fields",
                    "non-commit context control receipts cannot carry commit identity",
                ));
            }
        }
        if let Some(expected_boundary) = boundary {
            validate_boundary(expected_boundary)?;
            if self.boundary != *expected_boundary {
                return Err(ManagementError::conflict(
                    "context_control_receipt_boundary",
                    "context control receipt boundary does not match the request",
                ));
            }
        }
        Ok(())
    }
}
