// SPDX-License-Identifier: MIT

use super::*;

use crate::provider_session::{ContinuityMode, ProviderSessionMode};

/// Bounded metadata for one retained policy. Policy bytes and credential
/// references intentionally never cross the owner boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyMetadata {
    pub sha256: String,
    pub policy_id: String,
    pub version: u64,
    pub mode: ProviderSessionMode,
    pub continuity: ContinuityMode,
    pub max_completed_turns: usize,
    pub history_ttl_seconds: u64,
    pub epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyProposalMetadata {
    pub proposal_id: String,
    pub source_sha256: String,
    pub target_sha256: String,
    pub state: SessionPolicyMigrationState,
    pub approval_recorded: bool,
    pub adopted_policy_sha256: Option<String>,
}

/// Durable, redacted owner state suitable for a management projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyOwnerMetadata {
    pub scope: SessionScope,
    pub revision: u64,
    pub active: Option<ProviderSessionPolicyMetadata>,
    pub policies: Vec<ProviderSessionPolicyMetadata>,
    pub proposals: Vec<ProviderSessionPolicyProposalMetadata>,
}

impl ProviderSessionPolicyOwner {
    /// Returns bounded metadata only. Exact policy bytes, credential realm
    /// references and approval values remain private to the owner.
    pub fn metadata(
        &self,
    ) -> Result<ProviderSessionPolicyOwnerMetadata, ProviderSessionPolicyOwnerError> {
        let journal = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        let policies = journal
            .policies
            .iter()
            .map(policy_metadata)
            .collect::<Result<Vec<_>, _>>()?;
        let active = journal
            .active_sha256
            .as_ref()
            .map(|sha256| {
                journal
                    .policies
                    .iter()
                    .find(|record| &record.sha256 == sha256)
                    .ok_or(ProviderSessionPolicyOwnerError::Invalid)
                    .and_then(policy_metadata)
            })
            .transpose()?;
        let proposals = journal
            .proposals
            .iter()
            .map(|proposal| ProviderSessionPolicyProposalMetadata {
                proposal_id: proposal.id.clone(),
                source_sha256: proposal.source_sha256.clone(),
                target_sha256: proposal.target_sha256.clone(),
                state: proposal.migration.state,
                approval_recorded: proposal.migration.approval_ref.is_some(),
                adopted_policy_sha256: proposal.migration.adopted_policy_sha256.clone(),
            })
            .collect();
        Ok(ProviderSessionPolicyOwnerMetadata {
            scope: self.scope.clone(),
            revision: journal.revision,
            active,
            policies,
            proposals,
        })
    }
}

fn policy_metadata(
    record: &ProviderSessionPolicyRecord,
) -> Result<ProviderSessionPolicyMetadata, ProviderSessionPolicyOwnerError> {
    let policy: ProviderSessionPolicy = serde_json::from_slice(&record.bytes)
        .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
    policy
        .validate_schema()
        .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
    if crate::sha256_hex(&record.bytes) != record.sha256 {
        return Err(ProviderSessionPolicyOwnerError::Invalid);
    }
    Ok(ProviderSessionPolicyMetadata {
        sha256: record.sha256.clone(),
        policy_id: policy.policy_id,
        version: policy.version,
        mode: policy.mode,
        continuity: policy.continuity,
        max_completed_turns: policy.max_completed_turns,
        history_ttl_seconds: policy.history_ttl_seconds,
        epoch: policy.epoch,
    })
}
