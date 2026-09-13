// SPDX-License-Identifier: MIT

// Resumable migration state with an explicit downgrade fence.

pub const MEMORY_POLICY_MIGRATION_SCHEMA: &str =
    "ascension.context-memory.policy-migration.v1";
/// Maximum retained source-policy bytes in a migration record. This keeps exact-byte audit
/// history bounded even when a caller supplies formatting-heavy JSON.
pub const MEMORY_POLICY_MIGRATION_MAX_BYTES: usize = MAX_JOB_INPUT_BYTES;
/// The migration contract has four independently bounded policy limit fields.
pub const MEMORY_POLICY_MIGRATION_MAX_VIOLATIONS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMigrationState {
    Proposed,
    Approved,
    Adopted,
}

/// A bounded, reviewable migration proposal for a policy that is portable-schema valid but above
/// the selected owner/profile's effective limits. `new_from_bytes` retains the original policy
/// byte-for-byte; no value is clamped and no target policy is activated by this record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyMigrationProposal {
    pub schema: String,
    pub proposal_id: String,
    pub source_policy_id: String,
    pub source_policy_version: u64,
    pub source_policy_sha256: String,
    pub original_policy_bytes: Vec<u8>,
    pub target_capabilities_sha256: String,
    pub violations: Vec<MemoryLimitViolation>,
    pub state: PolicyMigrationState,
    pub approval_ref: Option<String>,
    pub adopted_policy_sha256: Option<String>,
}

impl PolicyMigrationProposal {
    pub fn new(
        policy: &MemoryPolicy,
        capabilities: &MemoryCapabilities,
        proposal_id: impl Into<String>,
    ) -> Result<Self, MemoryError> {
        let original_policy_bytes =
            serde_json::to_vec(policy).map_err(|_| MemoryError::InvalidProposal)?;
        Self::new_from_bytes(original_policy_bytes, capabilities, proposal_id)
    }

    /// Build a proposal from the exact bytes read from the policy store. This is the persistence
    /// path: formatting, field order, and unknown-but-schema-valid byte history are preserved
    /// instead of being regenerated from a Rust value.
    pub fn new_from_bytes(
        original_policy_bytes: impl AsRef<[u8]>,
        capabilities: &MemoryCapabilities,
        proposal_id: impl Into<String>,
    ) -> Result<Self, MemoryError> {
        let original_policy_bytes = original_policy_bytes.as_ref().to_vec();
        if original_policy_bytes.is_empty()
            || original_policy_bytes.len() > MEMORY_POLICY_MIGRATION_MAX_BYTES
        {
            return Err(MemoryError::InvalidProposal);
        }
        let policy: MemoryPolicy = serde_json::from_slice(&original_policy_bytes)
            .map_err(|_| MemoryError::InvalidProposal)?;
        policy.validate_schema()?;
        capabilities.validate()?;
        let proposal_id = proposal_id.into();
        let violations = policy.capability_limit_violations(capabilities);
        if !valid_id(&proposal_id) || violations.is_empty() {
            return Err(MemoryError::InvalidProposal);
        }
        let proposal = Self {
            schema: MEMORY_POLICY_MIGRATION_SCHEMA.to_owned(),
            proposal_id,
            source_policy_id: policy.policy_id.clone(),
            source_policy_version: policy.version,
            source_policy_sha256: sha256_hex(&original_policy_bytes),
            original_policy_bytes,
            target_capabilities_sha256: capabilities.binding.descriptor_sha256.clone(),
            violations,
            state: PolicyMigrationState::Proposed,
            approval_ref: None,
            adopted_policy_sha256: None,
        };
        proposal.validate()?;
        Ok(proposal)
    }

    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != MEMORY_POLICY_MIGRATION_SCHEMA
            || !valid_id(&self.proposal_id)
            || !valid_id(&self.source_policy_id)
            || self.source_policy_version == 0
            || !valid_digest(&self.source_policy_sha256)
            || self.original_policy_bytes.is_empty()
            || self.original_policy_bytes.len() > MEMORY_POLICY_MIGRATION_MAX_BYTES
            || sha256_hex(&self.original_policy_bytes) != self.source_policy_sha256
            || !valid_digest(&self.target_capabilities_sha256)
            || self.violations.is_empty()
            || self.violations.len() > MEMORY_POLICY_MIGRATION_MAX_VIOLATIONS
            || self
                .violations
                .iter()
                .map(|violation| violation.limit.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.violations.len()
            || self
                .violations
                .iter()
                .any(|violation| {
                    !valid_memory_violation(violation)
                        || violation.requested <= violation.effective
                        || violation.requested == 0
                        || violation.effective == 0
                })
            || matches!(self.state, PolicyMigrationState::Proposed)
                && self.approval_ref.is_some()
            || matches!(self.state, PolicyMigrationState::Approved | PolicyMigrationState::Adopted)
                && self.approval_ref.as_deref().is_none_or(|value| !valid_id(value))
            || matches!(self.state, PolicyMigrationState::Proposed | PolicyMigrationState::Approved)
                && self.adopted_policy_sha256.is_some()
            || self.state == PolicyMigrationState::Adopted
                && self.adopted_policy_sha256.is_none()
            || self
                .adopted_policy_sha256
                .as_deref()
                .is_some_and(|value| !valid_digest(value))
        {
            return Err(MemoryError::InvalidProposal);
        }
        let source: MemoryPolicy = serde_json::from_slice(&self.original_policy_bytes)
            .map_err(|_| MemoryError::InvalidProposal)?;
        source.validate_schema()?;
        if source.policy_id != self.source_policy_id || source.version != self.source_policy_version
        {
            return Err(MemoryError::InvalidProposal);
        }
        Ok(())
    }

    /// Record explicit operator approval. This does not modify the source bytes or activate a
    /// target policy.
    pub fn approve(&mut self, approval_ref: impl Into<String>) -> Result<(), MemoryError> {
        self.validate()?;
        let approval_ref = approval_ref.into();
        if self.state != PolicyMigrationState::Proposed || !valid_id(&approval_ref) {
            return Err(MemoryError::PermissionDenied);
        }
        self.approval_ref = Some(approval_ref);
        self.state = PolicyMigrationState::Approved;
        self.validate()
    }

    /// Explicitly adopt a caller-supplied target policy after approval. The target must already
    /// be bounded by the supplied owner descriptor; this API never derives a clamped target.
    pub fn adopt(
        &mut self,
        target: &MemoryPolicy,
        capabilities: &MemoryCapabilities,
        approval_ref: &str,
    ) -> Result<MemoryPolicy, MemoryError> {
        self.validate()?;
        capabilities.validate()?;
        if capabilities.binding.descriptor_sha256 != self.target_capabilities_sha256 {
            return Err(MemoryError::InvalidCapabilities);
        }
        if self.state != PolicyMigrationState::Approved
            || self.approval_ref.as_deref() != Some(approval_ref)
        {
            return Err(MemoryError::PermissionDenied);
        }
        target.validate_schema()?;
        if target.policy_id != self.source_policy_id
            || target.version <= self.source_policy_version
            || target.status != PolicyStatus::Approved
            || target.phase2_revision_id.is_none()
            || target.corpus_generation == 0
        {
            return Err(MemoryError::InvalidProposal);
        }
        if let Some(violation) = target.capability_limit_violations(capabilities).first() {
            return Err(MemoryError::CapabilityLimitExceeded {
                limit: violation.limit.clone(),
                requested: violation.requested,
                effective: violation.effective,
            });
        }
        let target_bytes = serde_json::to_vec(target).map_err(|_| MemoryError::InvalidProposal)?;
        self.adopted_policy_sha256 = Some(sha256_hex(target_bytes));
        self.state = PolicyMigrationState::Adopted;
        self.validate()?;
        Ok(target.clone())
    }

    #[must_use]
    pub fn original_policy_bytes(&self) -> &[u8] {
        &self.original_policy_bytes
    }
}

fn valid_memory_violation(violation: &MemoryLimitViolation) -> bool {
    match violation.limit.as_str() {
        "max_candidates" => {
            violation.requested <= MAX_CANDIDATES && violation.effective <= MAX_CANDIDATES
        }
        "max_results" => {
            violation.requested <= MAX_RESULTS && violation.effective <= MAX_RESULTS
        }
        "max_selected" => {
            violation.requested <= MAX_SELECTED && violation.effective <= MAX_SELECTED
        }
        "optional_byte_budget" => {
            violation.requested <= MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES
                && violation.effective <= MAX_OPTIONAL_BYTES
        }
        _ => false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationController {
    journal: MigrationJournal,
    total_checkpoints: u64,
    fail_next_step: bool,
}

impl MigrationController {
    pub fn new(journal: MigrationJournal, total_checkpoints: u64) -> Result<Self, MemoryError> {
        journal.validate()?;
        if total_checkpoints == 0 || journal.checkpoint > total_checkpoints {
            return Err(MemoryError::InvalidQuery);
        }
        Ok(Self {
            journal,
            total_checkpoints,
            fail_next_step: false,
        })
    }

    pub fn set_fail_next_step(&mut self, fail: bool) {
        self.fail_next_step = fail;
    }

    pub fn step(&mut self) -> Result<MigrationPhase, MemoryError> {
        if self.journal.phase == MigrationPhase::Complete {
            return Ok(MigrationPhase::Complete);
        }
        if self.fail_next_step {
            self.fail_next_step = false;
            self.journal.phase = MigrationPhase::Applying;
            return Err(MemoryError::PublicationFailed);
        }
        self.journal.phase = MigrationPhase::Applying;
        self.journal.checkpoint = self
            .journal
            .checkpoint
            .saturating_add(1)
            .min(self.total_checkpoints);
        if self.journal.checkpoint == self.total_checkpoints {
            self.journal.phase = MigrationPhase::Complete;
        }
        Ok(self.journal.phase)
    }

    pub fn resume(&mut self) -> Result<MigrationPhase, MemoryError> {
        self.step()
    }

    pub fn journal(&self) -> &MigrationJournal {
        &self.journal
    }
}
