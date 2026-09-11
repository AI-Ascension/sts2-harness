// SPDX-License-Identifier: MIT

// One-shot prepared-input consumption.  This stays beside the memory policy and records no
// provider or game effect; only the existing Phase 2 resume caller can use the returned bytes.

pub const MEMORY_RESUME_SCHEMA: &str = "ascension.context-memory.resume.v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedResume {
    pub schema: String,
    pub approval_id: String,
    pub selection_id: String,
    pub phase2_revision_id: String,
    pub prepared_manifest_sha256: String,
    pub revocation_epoch: u64,
    pub expires_at: String,
    #[serde(skip)]
    rendered_bytes: Vec<u8>,
    #[serde(skip)]
    consumed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeSubmission {
    pub approval_id: String,
    pub phase2_revision_id: String,
    pub prepared_manifest_sha256: String,
    pub rendered_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResumeOutcome {
    Submitted,
    AlreadySubmitted,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FirstResumeLedger {
    prepared: BTreeMap<String, PreparedResume>,
}

impl FirstResumeLedger {
    pub fn prepare(
        &mut self,
        approval: &MemoryApproval,
        selection: &SelectionManifest,
    ) -> Result<(), MemoryError> {
        if approval.state != ApprovalState::CommittedHeld
            || approval.selection_id != selection.selection_id
            || approval.selection_sha256 != selection.prepared_manifest_sha256
            || approval.phase2_prepared_manifest_sha256
                != selection.phase2_prepared_manifest_sha256
            || selection.phase2_revision_id != approval.planned_phase2_revision_id
            || selection.revocation_epoch != approval.revocation_epoch
            || !valid_timestamp(&selection.expires_at)
            || !valid_digest(&selection.prepared_manifest_sha256)
            || sha256_hex(&selection.rendered_bytes) != selection.prepared_manifest_sha256
        {
            return Err(MemoryError::StaleApproval);
        }
        let prepared = PreparedResume {
            schema: MEMORY_RESUME_SCHEMA.to_owned(),
            approval_id: approval.approval_id.clone(),
            selection_id: selection.selection_id.clone(),
            phase2_revision_id: selection.phase2_revision_id.clone(),
            prepared_manifest_sha256: selection.prepared_manifest_sha256.clone(),
            revocation_epoch: selection.revocation_epoch,
            expires_at: selection.expires_at.clone(),
            rendered_bytes: selection.rendered_bytes.clone(),
            consumed: false,
        };
        if let Some(existing) = self.prepared.get(&prepared.approval_id) {
            return if existing == &prepared {
                Ok(())
            } else {
                Err(MemoryError::Conflict)
            };
        }
        self.prepared.insert(prepared.approval_id.clone(), prepared);
        Ok(())
    }

    pub fn submit_first(
        &mut self,
        approval: &MemoryApproval,
        current_revocation_epoch: u64,
        now: &str,
        rendered_bytes: &[u8],
    ) -> Result<(ResumeOutcome, ResumeSubmission), MemoryError> {
        if !valid_timestamp(now) {
            return Err(MemoryError::InvalidQuery);
        }
        let prepared = self
            .prepared
            .get_mut(&approval.approval_id)
            .ok_or(MemoryError::StaleApproval)?;
        if approval.state != ApprovalState::CommittedHeld
            || approval.planned_phase2_revision_id != prepared.phase2_revision_id
            || approval.revocation_epoch != current_revocation_epoch
            || prepared.revocation_epoch != current_revocation_epoch
            || prepared.expires_at.as_str() <= now
            || sha256_hex(rendered_bytes) != prepared.prepared_manifest_sha256
            || rendered_bytes != prepared.rendered_bytes
        {
            return Err(MemoryError::StaleApproval);
        }
        if prepared.consumed {
            return Ok((
                ResumeOutcome::AlreadySubmitted,
                ResumeSubmission {
                    approval_id: prepared.approval_id.clone(),
                    phase2_revision_id: prepared.phase2_revision_id.clone(),
                    prepared_manifest_sha256: prepared.prepared_manifest_sha256.clone(),
                    rendered_bytes: Vec::new(),
                },
            ));
        }
        prepared.consumed = true;
        Ok((
            ResumeOutcome::Submitted,
            ResumeSubmission {
                approval_id: prepared.approval_id.clone(),
                phase2_revision_id: prepared.phase2_revision_id.clone(),
                prepared_manifest_sha256: prepared.prepared_manifest_sha256.clone(),
                rendered_bytes: prepared.rendered_bytes.clone(),
            },
        ))
    }

    pub fn is_consumed(&self, approval_id: &str) -> bool {
        self.prepared.get(approval_id).is_some_and(|item| item.consumed)
    }
}
