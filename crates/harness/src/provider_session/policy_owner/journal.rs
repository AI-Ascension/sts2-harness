// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn validate_journal(
    journal: &Journal,
    scope: &SessionScope,
    capabilities: &NativeCapabilities,
) -> Result<(), ProviderSessionPolicyOwnerError> {
    if journal.schema != SCHEMA
        || journal.revision == 0
        || journal.policies.len() > MAX_RECORDS
        || journal.proposals.len() > MAX_RECORDS
    {
        return Err(ProviderSessionPolicyOwnerError::Invalid);
    }
    let mut digests = std::collections::BTreeSet::new();
    for record in &journal.policies {
        if !digests.insert(record.sha256.clone()) {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
        let policy: ProviderSessionPolicy = serde_json::from_slice(&record.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        policy
            .validate_schema()
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        if policy.scope != *scope || crate::sha256_hex(&record.bytes) != record.sha256 {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
    }
    if let Some(active) = &journal.active_sha256 {
        let record = journal
            .policies
            .iter()
            .find(|record| &record.sha256 == active)
            .ok_or(ProviderSessionPolicyOwnerError::Invalid)?;
        let policy: ProviderSessionPolicy = serde_json::from_slice(&record.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        policy
            .admit_for_profile(capabilities)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
    }
    let mut proposal_ids = std::collections::BTreeSet::new();
    for proposal in &journal.proposals {
        let source = journal
            .policies
            .iter()
            .find(|record| record.sha256 == proposal.source_sha256)
            .ok_or(ProviderSessionPolicyOwnerError::Invalid)?;
        let source_policy: ProviderSessionPolicy = serde_json::from_slice(&source.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        let target = journal
            .policies
            .iter()
            .find(|record| record.sha256 == proposal.target_sha256)
            .ok_or(ProviderSessionPolicyOwnerError::Invalid)?;
        let target_policy: ProviderSessionPolicy = serde_json::from_slice(&target.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        let canonical_migration = SessionPolicyMigrationProposal::new_from_bytes(
            &source.bytes,
            capabilities,
            &proposal.id,
        )
        .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        let mut immutable_migration = proposal.migration.clone();
        immutable_migration.state = SessionPolicyMigrationState::Proposed;
        immutable_migration.approval_ref = None;
        immutable_migration.adopted_policy_sha256 = None;
        let expected_digest = serde_json::to_vec(&(
            &proposal.id,
            &proposal.source_sha256,
            &proposal.target_sha256,
            &immutable_migration,
        ))
        .ok()
        .map(crate::sha256_hex);
        if !proposal_ids.insert(proposal.id.clone())
            || !super::super::valid_id(&proposal.id)
            || expected_digest.as_deref() != Some(proposal.digest.as_str())
            || proposal.migration.validate().is_err()
            || proposal.migration.proposal_id != proposal.id
            || proposal.migration.source_policy_sha256 != proposal.source_sha256
            || proposal.migration.source_policy_id != source_policy.policy_id
            || proposal.migration.source_policy_version != source_policy.version
            || immutable_migration != canonical_migration
            || proposal.migration.target_capabilities_sha256
                != capabilities.binding.descriptor_sha256
            || target_policy.policy_id != source_policy.policy_id
            || target_policy.version <= source_policy.version
            || (proposal.migration.state == SessionPolicyMigrationState::Adopted
                && proposal.migration.adopted_policy_sha256.as_deref()
                    != Some(proposal.target_sha256.as_str()))
            || (proposal.migration.state == SessionPolicyMigrationState::Adopted
                && target_policy.admit_for_profile(capabilities).is_err())
        {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
    }
    Ok(())
}

pub(super) fn persist_candidate(
    store: &ProviderSessionMetadataStore,
    journal: &mut Journal,
    mut candidate: Journal,
) -> Result<(), ProviderSessionPolicyOwnerError> {
    candidate.revision = candidate
        .revision
        .checked_add(1)
        .ok_or(ProviderSessionPolicyOwnerError::Conflict)?;
    let bytes =
        serde_json::to_vec(&candidate).map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
    store
        .save_owner_journal(&bytes)
        .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
    *journal = candidate;
    Ok(())
}
