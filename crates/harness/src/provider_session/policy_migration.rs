// SPDX-License-Identifier: MIT

//! Bounded, reviewable migration records for saved provider-session policies.
//!
//! A saved policy can be valid against the portable contract and still exceed what the selected
//! adapter profile can execute (ADR 0026 classifies that difference). This module records the
//! difference instead of resolving it silently: a proposal retains the exact saved policy bytes,
//! lists the violated limits, and requires explicit operator approval before a caller-supplied
//! bounded target can be adopted. No value is ever clamped, rewritten or partially applied here.

use serde::{Deserialize, Serialize};

#[path = "policy_migration_adoption.rs"]
mod adoption;

use super::common::{
    MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS, SESSION_POLICY_SCHEMA,
    SESSION_POLICY_SCHEMA_MAX_COMPLETED_TURNS, SESSION_POLICY_SCHEMA_MAX_HISTORY_TTL_SECONDS,
    valid_digest, valid_id,
};
use super::{NativeCapabilities, ProviderSessionPolicy};

pub const SESSION_POLICY_MIGRATION_SCHEMA: &str = "ascension.provider-session.policy-migration.v1";
/// Maximum retained source-policy bytes. This keeps exact-byte audit history bounded even when a
/// caller supplies formatting-heavy JSON.
pub const SESSION_POLICY_MIGRATION_MAX_BYTES: usize = 16_384;
/// The provider-session policy contract has two independently bounded limit fields.
pub const SESSION_POLICY_MIGRATION_MAX_VIOLATIONS: usize = 2;

pub(crate) const TURN_LIMIT: &str = "max_completed_turns";
pub(crate) const TTL_LIMIT: &str = "max_history_ttl_seconds";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPolicyMigrationState {
    Proposed,
    Approved,
    Adopted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPolicyLimitViolation {
    pub limit: String,
    pub requested: u64,
    pub effective: u64,
}

impl SessionPolicyLimitViolation {
    #[must_use]
    fn valid(&self) -> bool {
        if self.requested == 0 || self.effective == 0 || self.requested <= self.effective {
            return false;
        }
        match self.limit.as_str() {
            TURN_LIMIT => {
                self.requested <= SESSION_POLICY_SCHEMA_MAX_COMPLETED_TURNS as u64
                    && self.effective <= MAX_COMPLETED_TURNS as u64
            }
            TTL_LIMIT => {
                self.requested <= SESSION_POLICY_SCHEMA_MAX_HISTORY_TTL_SECONDS
                    && self.effective <= MAX_HISTORY_TTL_SECONDS
            }
            _ => false,
        }
    }
}

/// A bounded, reviewable migration proposal for a saved policy that is portable-schema valid but
/// above the selected profile's effective limits.
///
/// The original policy is retained byte-for-byte and no target policy is derived, clamped or
/// activated by this record. Adoption requires an explicit approval naming this exact proposal and
/// a caller-supplied target that is itself within the executable ceilings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPolicyMigrationProposal {
    pub schema: String,
    pub proposal_id: String,
    pub source_policy_id: String,
    pub source_policy_version: u64,
    pub source_policy_sha256: String,
    pub original_policy_bytes: Vec<u8>,
    pub target_capabilities_sha256: String,
    pub violations: Vec<SessionPolicyLimitViolation>,
    pub state: SessionPolicyMigrationState,
    pub approval_ref: Option<String>,
    pub adopted_policy_sha256: Option<String>,
}

impl SessionPolicyMigrationProposal {
    /// Build a proposal from a parsed policy. Prefer [`Self::new_from_bytes`] on the persistence
    /// path so that original formatting and byte history are preserved.
    pub fn new(
        policy: &ProviderSessionPolicy,
        capabilities: &NativeCapabilities,
        proposal_id: impl Into<String>,
    ) -> Result<Self, SessionPolicyMigrationError> {
        let bytes =
            serde_json::to_vec(policy).map_err(|_| SessionPolicyMigrationError::InvalidProposal)?;
        Self::new_from_bytes(bytes, capabilities, proposal_id)
    }

    /// Build a proposal from the exact bytes read from the policy store.
    pub fn new_from_bytes(
        original_policy_bytes: impl AsRef<[u8]>,
        capabilities: &NativeCapabilities,
        proposal_id: impl Into<String>,
    ) -> Result<Self, SessionPolicyMigrationError> {
        let original_policy_bytes = original_policy_bytes.as_ref().to_vec();
        if original_policy_bytes.is_empty()
            || original_policy_bytes.len() > SESSION_POLICY_MIGRATION_MAX_BYTES
        {
            return Err(SessionPolicyMigrationError::InvalidProposal);
        }
        let policy: ProviderSessionPolicy = serde_json::from_slice(&original_policy_bytes)
            .map_err(|_| SessionPolicyMigrationError::InvalidProposal)?;
        policy
            .validate_schema()
            .map_err(|_| SessionPolicyMigrationError::InvalidProposal)?;
        let proposal_id = proposal_id.into();
        let violations = policy.capability_limit_violations(capabilities);
        if !valid_id(&proposal_id) || violations.is_empty() {
            return Err(SessionPolicyMigrationError::InvalidProposal);
        }
        let proposal = Self {
            schema: SESSION_POLICY_MIGRATION_SCHEMA.to_owned(),
            proposal_id,
            source_policy_id: policy.policy_id.clone(),
            source_policy_version: policy.version,
            source_policy_sha256: crate::sha256_hex(&original_policy_bytes),
            original_policy_bytes,
            target_capabilities_sha256: capabilities.binding.descriptor_sha256.clone(),
            violations,
            state: SessionPolicyMigrationState::Proposed,
            approval_ref: None,
            adopted_policy_sha256: None,
        };
        proposal.validate()?;
        Ok(proposal)
    }

    pub fn validate(&self) -> Result<(), SessionPolicyMigrationError> {
        let distinct_limits = self
            .violations
            .iter()
            .map(|violation| violation.limit.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if self.schema != SESSION_POLICY_MIGRATION_SCHEMA
            || !valid_id(&self.proposal_id)
            || !valid_id(&self.source_policy_id)
            || self.source_policy_version == 0
            || !valid_digest(&self.source_policy_sha256)
            || self.original_policy_bytes.is_empty()
            || self.original_policy_bytes.len() > SESSION_POLICY_MIGRATION_MAX_BYTES
            || crate::sha256_hex(&self.original_policy_bytes) != self.source_policy_sha256
            || !valid_digest(&self.target_capabilities_sha256)
            || self.violations.is_empty()
            || self.violations.len() > SESSION_POLICY_MIGRATION_MAX_VIOLATIONS
            || distinct_limits.len() != self.violations.len()
            || self.violations.iter().any(|violation| !violation.valid())
            || matches!(self.state, SessionPolicyMigrationState::Proposed)
                && self.approval_ref.is_some()
            || matches!(
                self.state,
                SessionPolicyMigrationState::Approved | SessionPolicyMigrationState::Adopted
            ) && self
                .approval_ref
                .as_deref()
                .is_none_or(|value| !valid_id(value))
            || matches!(
                self.state,
                SessionPolicyMigrationState::Proposed | SessionPolicyMigrationState::Approved
            ) && self.adopted_policy_sha256.is_some()
            || matches!(self.state, SessionPolicyMigrationState::Adopted)
                && self.adopted_policy_sha256.is_none()
            || self
                .adopted_policy_sha256
                .as_deref()
                .is_some_and(|value| !valid_digest(value))
        {
            return Err(SessionPolicyMigrationError::InvalidProposal);
        }
        let source: ProviderSessionPolicy = serde_json::from_slice(&self.original_policy_bytes)
            .map_err(|_| SessionPolicyMigrationError::InvalidProposal)?;
        source
            .validate_schema()
            .map_err(|_| SessionPolicyMigrationError::InvalidProposal)?;
        if source.policy_id != self.source_policy_id
            || source.version != self.source_policy_version
            || SESSION_POLICY_SCHEMA != source.schema.as_str()
        {
            return Err(SessionPolicyMigrationError::InvalidProposal);
        }
        Ok(())
    }

    /// Record explicit operator approval. This does not modify the source bytes, clamp a value, or
    /// activate a target policy.
    pub fn approve(
        &mut self,
        approval_ref: impl Into<String>,
    ) -> Result<(), SessionPolicyMigrationError> {
        self.validate()?;
        let approval_ref = approval_ref.into();
        if self.state != SessionPolicyMigrationState::Proposed || !valid_id(&approval_ref) {
            return Err(SessionPolicyMigrationError::PermissionDenied);
        }
        self.approval_ref = Some(approval_ref);
        self.state = SessionPolicyMigrationState::Approved;
        self.validate()
    }

    fn adopted_source_epoch(&self) -> u64 {
        serde_json::from_slice::<ProviderSessionPolicy>(&self.original_policy_bytes)
            .map(|policy| policy.epoch)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn original_policy_bytes(&self) -> &[u8] {
        &self.original_policy_bytes
    }
}

/// Why a saved-policy migration record could not be built, approved or adopted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionPolicyMigrationError {
    /// The record is malformed, the source policy is not portable-valid, or no limit is violated.
    InvalidProposal,
    /// The caller did not hold the required explicit approval for this exact record.
    PermissionDenied,
    /// The capability descriptor does not match the one the proposal was raised against.
    InvalidCapabilities,
    /// The supplied target still exceeds an executable limit; no clamped target is derived.
    CapabilityLimitExceeded {
        limit: String,
        requested: u64,
        effective: u64,
    },
}

impl SessionPolicyMigrationError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidProposal => "provider_session_policy_migration_invalid",
            Self::PermissionDenied => "provider_session_policy_migration_denied",
            Self::InvalidCapabilities => "provider_session_policy_migration_capabilities",
            Self::CapabilityLimitExceeded { .. } => "effective_limit_exceeded",
        }
    }
}
