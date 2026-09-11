// SPDX-License-Identifier: MIT

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
        let approval = self
            .approvals
            .get_mut(approval_id)
            .ok_or(MemoryError::StaleApproval)?;
        if approval.revocation_epoch != current_revocation_epoch
            || approval.expires_at.as_str() <= now
            || approval.state != ApprovalState::CommittedHeld
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryCapabilities {
    pub schema: String,
    pub product_phase: u8,
    pub scope: MemoryScope,
    pub enabled: bool,
    pub local_lexical_retrieval: String,
    pub extractive_compaction: String,
    pub abstractive_adapter: String,
    pub abstractive_live_verified: bool,
    pub per_decision_policy: String,
    pub phase2_approval_required: bool,
    pub persistent_provider_sessions: bool,
    pub provider_side_compaction: bool,
    pub semantic_vector_retrieval: String,
    pub hidden_reasoning_access: bool,
    pub direct_game_dispatch: bool,
    pub supported_operations: Vec<String>,
}

impl MemoryCorpus {
    pub fn capabilities(&self) -> MemoryCapabilities {
        MemoryCapabilities {
            schema: MEMORY_CAPABILITIES_SCHEMA.to_owned(),
            product_phase: 3,
            scope: self.scope.clone(),
            enabled: self.enabled,
            local_lexical_retrieval: "supported".to_owned(),
            extractive_compaction: "supported".to_owned(),
            abstractive_adapter: "supported".to_owned(),
            abstractive_live_verified: false,
            per_decision_policy: "supported".to_owned(),
            phase2_approval_required: true,
            persistent_provider_sessions: false,
            provider_side_compaction: false,
            semantic_vector_retrieval: "unsupported".to_owned(),
            hidden_reasoning_access: false,
            direct_game_dispatch: false,
            supported_operations: if self.enabled {
                vec![
                    "search", "extract", "generate", "review", "select", "policy", "adopt",
                    "revoke", "evaluate",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect()
            } else {
                Vec::new()
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum MemoryRole {
    Read,
    Search,
    Generate,
    Review,
    Select,
    Policy,
    Revoke,
    Control,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryAuthorizer {
    grants: BTreeMap<String, BTreeSet<MemoryRole>>,
}

impl MemoryAuthorizer {
    pub fn new() -> Self {
        Self {
            grants: BTreeMap::new(),
        }
    }

    pub fn grant(
        &mut self,
        principal: impl Into<String>,
        role: MemoryRole,
    ) -> Result<(), MemoryError> {
        let principal = principal.into();
        if !valid_id(&principal) {
            return Err(MemoryError::PermissionDenied);
        }
        self.grants.entry(principal).or_default().insert(role);
        Ok(())
    }

    pub fn check(&self, principal: &str, role: MemoryRole) -> Result<(), MemoryError> {
        if self
            .grants
            .get(principal)
            .is_some_and(|roles| roles.contains(&role))
        {
            Ok(())
        } else {
            Err(MemoryError::PermissionDenied)
        }
    }
}

impl Default for MemoryAuthorizer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
