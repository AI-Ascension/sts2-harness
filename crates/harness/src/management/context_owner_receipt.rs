// SPDX-License-Identifier: MIT

//! Exact command and receipt records for delegated context control.
//!
//! Receipt v2 is intentionally incompatible with v1: v1 did not carry the
//! command variant or resulting boundary, so a reader cannot reconstruct the
//! transition identity. V1 receipts are rejected with a schema error and must
//! be reissued as v2. A rollback to a v1 reader is safe only after v2 receipts
//! have been drained; no v2 receipt may be downgraded or replayed as v1.

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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
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
        if self.schema_version != CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION {
            return Err(ManagementError::conflict(
                "context_control_receipt_schema_unsupported",
                format!(
                    "context control receipt schema {} is unsupported; expected {}; \
                     v1 receipts cannot be upgraded safely and must be reissued",
                    self.schema_version, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION
                ),
            ));
        }
        binding.validate(None)?;
        validate_identifier("context_control_command_id", &self.command_id)?;
        validate_identifier("context_control_idempotency_key", &self.idempotency_key)?;
        validate_identifier("context_control_effect", &self.effect)?;
        validate_digest("context_binding_digest", &self.binding_digest)?;
        let expectation = CommandExpectation::from(binding, command)?;
        if self.owner_id != binding.owner_id
            || self.invocation_id != binding.invocation_id
            || self.binding_id != binding.binding_id
            || self.binding_digest != binding.binding_digest
            || self.command != expectation.kind
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_mismatch",
                "context control receipt is not bound to the requested owner invocation",
            ));
        }
        validate_boundary(&self.boundary)?;
        if self.boundary != expectation.resulting_boundary
            || self.controller_epoch != expectation.resulting_boundary.controller_epoch
            || self.gate_epoch != expectation.resulting_boundary.gate_epoch
            || self.control_version != expectation.resulting_boundary.control_version
            || self.plan_epoch != expectation.resulting_plan_epoch
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_transition",
                "context control receipt does not describe the command's resulting state",
            ));
        }
        let expected_effect = match expectation.kind {
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
        if self.idempotency_key != expectation.idempotency_key {
            return Err(ManagementError::conflict(
                "context_control_receipt_idempotency",
                "context control receipt does not match the requested idempotency key",
            ));
        }
        match expectation.commit_identity {
            Some((expected_revision_id, preview, approved)) => {
                if self.revision_id.as_deref() == Some(expected_revision_id.as_str()) {
                    return Err(ManagementError::conflict(
                        "context_control_receipt_revision",
                        "a commit receipt must identify a revision newer than its pre-command fence",
                    ));
                }
                let revision = self.revision_id.as_deref().ok_or_else(|| {
                    ManagementError::conflict(
                        "context_control_receipt_revision",
                        "a commit receipt must carry its resulting revision identity",
                    )
                })?;
                validate_identifier("context_control_revision_id", revision)?;
                validate_digest("context_control_preview_manifest", preview.as_str())?;
                validate_digest("context_control_approved_manifest", approved.as_str())?;
                if self.preview_manifest_digest.as_deref() != Some(preview.as_str())
                    || self.approved_manifest_digest.as_deref() != Some(approved.as_str())
                {
                    return Err(ManagementError::conflict(
                        "context_control_receipt_commit_identity",
                        "context control commit receipt does not match its manifest identities",
                    ));
                }
            }
            None if self.revision_id.is_none()
                && self.preview_manifest_digest.is_none()
                && self.approved_manifest_digest.is_none() => {}
            None => {
                return Err(ManagementError::conflict(
                    "context_control_receipt_commit_fields",
                    "non-commit context control receipts cannot carry commit identity",
                ));
            }
        }
        Ok(())
    }
}

#[path = "context_owner_receipt_decode.rs"]
mod decode;

struct CommandExpectation {
    kind: ContextControlCommandKind,
    idempotency_key: String,
    resulting_plan_epoch: u64,
    resulting_boundary: ContextBoundary,
    commit_identity: Option<(String, String, String)>,
}

impl CommandExpectation {
    fn from(
        binding: &ContextOwnerBinding,
        command: &ContextControlCommand,
    ) -> Result<Self, ManagementError> {
        match command {
            ContextControlCommand::Pause {
                idempotency_key,
                expected_control_version,
            } => {
                validate_identifier("context_control_idempotency_key", idempotency_key)?;
                ensure_pre_control(binding, *expected_control_version)?;
                Ok(Self {
                    kind: ContextControlCommandKind::Pause,
                    idempotency_key: idempotency_key.clone(),
                    resulting_plan_epoch: binding.plan_epoch,
                    resulting_boundary: transition_boundary(&binding.boundary, true)?,
                    commit_identity: None,
                })
            }
            ContextControlCommand::Commit {
                idempotency_key,
                expected_control_version,
                expected_revision_id,
                expected_boundary,
                preview_manifest_digest,
                approved_manifest_digest,
            } => {
                validate_identifier("context_control_idempotency_key", idempotency_key)?;
                validate_identifier("context_control_revision_id", expected_revision_id)?;
                validate_digest("context_control_preview_manifest", preview_manifest_digest)?;
                validate_digest(
                    "context_control_approved_manifest",
                    approved_manifest_digest,
                )?;
                ensure_pre_state(binding, *expected_control_version, expected_boundary)?;
                if expected_revision_id != &binding.approved_revision_id {
                    return Err(ManagementError::conflict(
                        "context_control_revision_fence",
                        "commit command revision fence is not attached to the authoritative binding",
                    ));
                }
                if preview_manifest_digest != approved_manifest_digest {
                    return Err(ManagementError::conflict(
                        "context_control_preview",
                        "commit command preview and approved manifests differ",
                    ));
                }
                Ok(Self {
                    kind: ContextControlCommandKind::Commit,
                    idempotency_key: idempotency_key.clone(),
                    resulting_plan_epoch: checked_increment(binding.plan_epoch, "plan_epoch")?,
                    resulting_boundary: transition_boundary(expected_boundary, false)?,
                    commit_identity: Some((
                        expected_revision_id.clone(),
                        preview_manifest_digest.clone(),
                        approved_manifest_digest.clone(),
                    )),
                })
            }
            ContextControlCommand::Resume {
                idempotency_key,
                expected_control_version,
                expected_boundary,
            } => {
                validate_identifier("context_control_idempotency_key", idempotency_key)?;
                ensure_pre_state(binding, *expected_control_version, expected_boundary)?;
                Ok(Self {
                    kind: ContextControlCommandKind::Resume,
                    idempotency_key: idempotency_key.clone(),
                    resulting_plan_epoch: binding.plan_epoch,
                    resulting_boundary: transition_boundary(expected_boundary, true)?,
                    commit_identity: None,
                })
            }
        }
    }
}

fn ensure_pre_control(
    binding: &ContextOwnerBinding,
    expected_control_version: u64,
) -> Result<(), ManagementError> {
    if expected_control_version == 0 || expected_control_version != binding.boundary.control_version
    {
        return Err(ManagementError::conflict(
            "context_control_pre_fence",
            "command control version is not the authoritative pre-command fence",
        ));
    }
    Ok(())
}

fn ensure_pre_state(
    binding: &ContextOwnerBinding,
    expected_control_version: u64,
    expected_boundary: &ContextBoundary,
) -> Result<(), ManagementError> {
    ensure_pre_control(binding, expected_control_version)?;
    validate_boundary(expected_boundary)?;
    if expected_boundary != &binding.boundary {
        return Err(ManagementError::conflict(
            "context_control_pre_boundary",
            "command boundary is not the authoritative pre-command boundary",
        ));
    }
    Ok(())
}

fn transition_boundary(
    pre: &ContextBoundary,
    advances_gate: bool,
) -> Result<ContextBoundary, ManagementError> {
    let mut resulting = pre.clone();
    resulting.control_version = checked_increment(pre.control_version, "control_version")?;
    if advances_gate {
        resulting.gate_epoch = checked_increment(pre.gate_epoch, "gate_epoch")?;
    }
    Ok(resulting)
}

fn checked_increment(value: u64, field: &str) -> Result<u64, ManagementError> {
    value.checked_add(1).ok_or_else(|| {
        ManagementError::budget(
            "context_control_epoch_exhausted",
            format!("context control {field} cannot advance"),
        )
    })
}
