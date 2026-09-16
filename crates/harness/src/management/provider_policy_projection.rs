// SPDX-License-Identifier: MIT

use crate::provider_session::{
    ProviderSessionPolicyMetadata, ProviderSessionPolicyOwnerMetadata,
    ProviderSessionPolicyProposalMetadata, SessionPolicyMigrationState,
};

use super::super::contract::{
    ProviderSessionPolicyBindingMetadata, ProviderSessionPolicyHistoryMetadata,
    ProviderSessionPolicyProposalMetadata as ApiProposalMetadata, ProviderSessionPolicyViewValue,
};
use super::super::{ManagementError, RunSnapshot};

pub(super) fn view_value(
    metadata: ProviderSessionPolicyOwnerMetadata,
    snapshot: &RunSnapshot,
) -> Result<ProviderSessionPolicyViewValue, ManagementError> {
    if metadata.scope.run_id != snapshot.workflow_run_id {
        return Err(ManagementError::conflict(
            "provider_session_policy_scope_mismatch",
            "provider-session policy owner is scoped to a different workflow run",
        ));
    }
    let active_sha256 = metadata
        .active
        .as_ref()
        .map(|active| active.sha256.as_str());
    let history = metadata
        .policies
        .iter()
        .map(|policy| history_metadata(policy, active_sha256))
        .collect();
    Ok(ProviderSessionPolicyViewValue {
        run_id: snapshot.workflow_run_id.clone(),
        revision: metadata.revision,
        active: metadata.active.map(binding_metadata),
        history,
        proposals: metadata
            .proposals
            .into_iter()
            .map(proposal_metadata)
            .collect(),
    })
}

fn binding_metadata(policy: ProviderSessionPolicyMetadata) -> ProviderSessionPolicyBindingMetadata {
    ProviderSessionPolicyBindingMetadata {
        sha256: policy.sha256,
        policy_id: policy.policy_id,
        version: policy.version,
        mode: mode_name(policy.mode).to_owned(),
        continuity: continuity_name(policy.continuity).to_owned(),
        max_completed_turns: policy.max_completed_turns,
        history_ttl_seconds: policy.history_ttl_seconds,
        epoch: policy.epoch,
    }
}

fn history_metadata(
    policy: &ProviderSessionPolicyMetadata,
    active_sha256: Option<&str>,
) -> ProviderSessionPolicyHistoryMetadata {
    ProviderSessionPolicyHistoryMetadata {
        sha256: policy.sha256.clone(),
        policy_id: policy.policy_id.clone(),
        version: policy.version,
        mode: mode_name(policy.mode).to_owned(),
        continuity: continuity_name(policy.continuity).to_owned(),
        active: active_sha256 == Some(policy.sha256.as_str()),
    }
}

fn proposal_metadata(proposal: ProviderSessionPolicyProposalMetadata) -> ApiProposalMetadata {
    ApiProposalMetadata {
        proposal_id: proposal.proposal_id,
        proposal_sha256: proposal.proposal_sha256,
        source_sha256: proposal.source_sha256,
        target_sha256: proposal.target_sha256,
        state: match proposal.state {
            SessionPolicyMigrationState::Proposed => "proposed",
            SessionPolicyMigrationState::Approved => "approved",
            SessionPolicyMigrationState::Adopted => "adopted",
        }
        .to_owned(),
        approval_recorded: proposal.approval_recorded,
        adopted_policy_sha256: proposal.adopted_policy_sha256,
    }
}

fn mode_name(mode: crate::provider_session::ProviderSessionMode) -> &'static str {
    match mode {
        crate::provider_session::ProviderSessionMode::Disabled => "disabled",
        crate::provider_session::ProviderSessionMode::FixtureOnly => "fixture_only",
        crate::provider_session::ProviderSessionMode::InspectOnly => "inspect_only",
        crate::provider_session::ProviderSessionMode::Enabled => "enabled",
    }
}

fn continuity_name(mode: crate::provider_session::ContinuityMode) -> &'static str {
    match mode {
        crate::provider_session::ContinuityMode::StrictReviewed => "strict_reviewed",
        crate::provider_session::ContinuityMode::ObservedPersistent => "observed_persistent",
    }
}
