// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    PreviewReady,
    CommittedHeld,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryApproval {
    pub schema: String,
    pub approval_id: String,
    pub scope: MemoryScope,
    pub selection_id: String,
    pub selection_sha256: String,
    pub policy_id: String,
    pub policy_version: u64,
    pub phase2_preview_id: String,
    pub phase2_prepared_manifest_sha256: String,
    pub planned_phase2_revision_id: String,
    pub source_manifest_sha256: String,
    pub revocation_epoch: u64,
    pub state: ApprovalState,
    pub commit_auto_resumes: bool,
    pub summary_inference_calls: u32,
    pub gameplay_inference_calls: u32,
    pub expires_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalStore {
    approvals: BTreeMap<String, MemoryApproval>,
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self {
            approvals: BTreeMap::new(),
        }
    }

    pub fn bind(
        &mut self,
        approval_id: impl Into<String>,
        selection: &SelectionManifest,
        phase2_preview_id: impl Into<String>,
        source_manifest_sha256: impl Into<String>,
    ) -> Result<MemoryApproval, MemoryError> {
        let approval = MemoryApproval {
            schema: MEMORY_APPROVAL_SCHEMA.to_owned(),
            approval_id: approval_id.into(),
            scope: selection.scope.clone(),
            selection_id: selection.selection_id.clone(),
            selection_sha256: sha256_hex(&selection.rendered_bytes),
            policy_id: selection.policy_id.clone(),
            policy_version: selection.policy_version,
            phase2_preview_id: phase2_preview_id.into(),
            phase2_prepared_manifest_sha256: selection.phase2_prepared_manifest_sha256.clone(),
            planned_phase2_revision_id: selection.phase2_revision_id.clone(),
            source_manifest_sha256: source_manifest_sha256.into(),
            revocation_epoch: selection.revocation_epoch,
            state: ApprovalState::PreviewReady,
            commit_auto_resumes: false,
            summary_inference_calls: 0,
            gameplay_inference_calls: 0,
            expires_at: selection.expires_at.clone(),
        };
        if !valid_id(&approval.approval_id)
            || !valid_id(&approval.phase2_preview_id)
            || !valid_digest(&approval.source_manifest_sha256)
            || !valid_digest(&selection.prepared_manifest_sha256)
            || sha256_hex(&selection.rendered_bytes) != selection.prepared_manifest_sha256
            || !valid_digest(&selection.phase2_prepared_manifest_sha256)
            || !valid_id(&approval.planned_phase2_revision_id)
            || !valid_timestamp(&approval.expires_at)
            || approval.source_manifest_sha256 != source_manifest_digest(&selection.selected_sources)
        {
            return Err(MemoryError::InvalidQuery);
        }
        if let Some(existing) = self.approvals.get(&approval.approval_id) {
            return if existing == &approval {
                Ok(existing.clone())
            } else {
                Err(MemoryError::Conflict)
            };
        }
        self.approvals
            .insert(approval.approval_id.clone(), approval.clone());
        Ok(approval)
    }

    pub fn commit_held(
        &mut self,
        approval_id: &str,
        current_revocation_epoch: u64,
        now: &str,
    ) -> Result<MemoryApproval, MemoryError> {
        if !valid_timestamp(now) {
            return Err(MemoryError::InvalidQuery);
        }
        let approval = self
            .approvals
            .get_mut(approval_id)
            .ok_or(MemoryError::StaleApproval)?;

        if approval.revocation_epoch != current_revocation_epoch
            || approval.expires_at.as_str() <= now
            || approval.state != ApprovalState::PreviewReady
        {
            approval.state = ApprovalState::Stale;
            return Err(MemoryError::StaleApproval);
        }
        approval.state = ApprovalState::CommittedHeld;
        Ok(approval.clone())
    }

    pub fn explicit_resume(
        &mut self,
        approval_id: &str,
        current_revocation_epoch: u64,
        now: &str,
    ) -> Result<MemoryApproval, MemoryError> {
        if !valid_timestamp(now) {
            return Err(MemoryError::InvalidQuery);
        }
        let approval = self
            .approvals
            .get_mut(approval_id)
            .ok_or(MemoryError::StaleApproval)?;
        if approval.revocation_epoch != current_revocation_epoch
            || approval.expires_at.as_str() <= now
            || approval.state != ApprovalState::CommittedHeld
            || approval.gameplay_inference_calls != 0
        {
            approval.state = ApprovalState::Stale;
            return Err(MemoryError::StaleApproval);
        }
        approval.gameplay_inference_calls = 1;
        Ok(approval.clone())
    }

    pub fn invalidate_revoked(&mut self, epoch: u64) {
        for approval in self.approvals.values_mut() {
            if approval.revocation_epoch < epoch && approval.state != ApprovalState::Stale {
                approval.state = ApprovalState::Stale;
            }
        }
    }

    pub fn approval(&self, approval_id: &str) -> Option<&MemoryApproval> {
        self.approvals.get(approval_id)
    }
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStatus {
    Pending,
    Running,
    Complete,
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevocationRecord {
    pub schema: String,
    pub revocation_id: String,
    pub scope: MemoryScope,
    pub roots: Vec<MemoryRef>,
    pub revocation_epoch: u64,
    pub denial_committed: bool,
    pub cleanup_status: CleanupStatus,
    pub affected_derivatives: usize,
    pub historical_manifests_rewritten: bool,
    pub created_at: String,
}

impl RevocationRecord {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != MEMORY_REVOCATION_SCHEMA
            || !valid_id(&self.revocation_id)
            || !self.scope.valid()
            || self.roots.is_empty()
            || self.roots.len() > 16
            || self.roots.iter().any(|root| !root.valid())
            || self.roots.iter().collect::<BTreeSet<_>>().len() != self.roots.len()
            || self.revocation_epoch == 0
            || !self.denial_committed
            || !valid_timestamp(&self.created_at)
        {
            return Err(MemoryError::InvalidEntry);
        }
        Ok(())
    }
}
