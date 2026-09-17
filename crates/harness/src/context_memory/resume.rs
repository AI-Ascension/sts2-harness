// SPDX-License-Identifier: MIT

// One-shot prepared-input consumption.  This stays beside the memory policy and records no
// provider or game effect; only the existing Phase 2 resume caller can use the returned bytes.

pub const MEMORY_RESUME_SCHEMA: &str = "ascension.context-memory.resume.v1";

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
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

// Debug is allowlisted. `rendered_bytes` is the one-shot prepared input this module exists to
// consume; it is deliberately non-public, and `#[serde(skip)]` keeps it out of serialized output,
// but serde attributes do not apply to Debug, so the derive published it. Format only the fixed
// schema, the revocation epoch and the shape; the prepared bytes, the content digest and the
// caller-controlled identifiers stay out of both ordinary and alternate formatting.
impl std::fmt::Debug for PreparedResume {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedResume")
            .field("schema", &self.schema)
            .field("revocation_epoch", &self.revocation_epoch)
            .field("rendered_byte_count", &self.rendered_bytes.len())
            .field("consumed", &self.consumed)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ResumeSubmission {
    pub approval_id: String,
    pub phase2_revision_id: String,
    pub prepared_manifest_sha256: String,
    pub rendered_bytes: Vec<u8>,
}

// `ResumeSubmission` hands the same prepared bytes back to the single Phase 2 caller. Keep the
// bytes and the caller-controlled identifiers out of both ordinary and alternate formatting, and
// finish non-exhaustively so a later sensitive field cannot silently re-enter the format path.
impl std::fmt::Debug for ResumeSubmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResumeSubmission")
            .field("rendered_byte_count", &self.rendered_bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResumeOutcome {
    Submitted,
    AlreadySubmitted,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct FirstResumeLedger {
    prepared: BTreeMap<String, PreparedResume>,
}

// The ledger derives Debug over a map of `PreparedResume`, so the derived recursion formatted
// every held prepared input. Report only the count instead of recursing into the entries.
impl std::fmt::Debug for FirstResumeLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FirstResumeLedger")
            .field("prepared_count", &self.prepared.len())
            .finish_non_exhaustive()
    }
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
